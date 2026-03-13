//! Seccomp-BPF sandbox for restricting syscalls after the Flash plugin is loaded.
//!
//! Once activated, this blocks:
//! - `execve` / `execveat` — prevents spawning child processes (`system()` uses `execve`)
//! - `mmap` with `PROT_EXEC` — prevents mapping new executable memory (blocks `dlopen`)
//! - `memfd_create` — prevents creating anonymous executable files
//!
//! Note: `mprotect` with `PROT_EXEC` is intentionally **allowed** so that Flash's
//! AVM2 JIT compiler can transition RW pages to RX (W^X pattern).
//!
//! The sandbox uses seccomp in filter mode (SECCOMP_SET_MODE_FILTER) with a BPF
//! program. Blocked syscalls return `EPERM`.

#[cfg(target_os = "linux")]
mod inner {
    use std::io;

    // -----------------------------------------------------------------------
    // Constants from <linux/seccomp.h>, <linux/filter.h>, <linux/bpf_common.h>,
    // <linux/audit.h>, and <asm/unistd_64.h>.
    // -----------------------------------------------------------------------

    const SECCOMP_SET_MODE_FILTER: libc::c_uint = 1;

    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

    // BPF instruction classes and modifiers
    const BPF_LD: u16 = 0x00;
    const BPF_JMP: u16 = 0x05;
    const BPF_RET: u16 = 0x06;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_AND: u16 = 0x50;
    const BPF_ALU: u16 = 0x04;

    // Offsets into seccomp_data  (see <linux/seccomp.h>)
    const SECCOMP_DATA_NR: u32 = 0; // offsetof(struct seccomp_data, nr)
    const SECCOMP_DATA_ARCH: u32 = 4; // offsetof(struct seccomp_data, arch)
    // args[N] starts at offset 16; each arg is u64 (8 bytes)
    const SECCOMP_DATA_ARG_OFFSET: u32 = 16;

    // Architecture audit value for x86-64
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;

    // Syscall numbers (x86-64)
    const SYS_MMAP: u32 = 9;
    const SYS_EXECVE: u32 = 59;
    const SYS_EXECVEAT: u32 = 322;
    const SYS_MEMFD_CREATE: u32 = 319;

    // mmap prot arg is arg index 2; mprotect prot arg is arg index 2
    const PROT_EXEC: u32 = 0x4;

    /// A single BPF instruction (matches `struct sock_filter`).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    impl SockFilter {
        const fn new(code: u16, jt: u8, jf: u8, k: u32) -> Self {
            Self { code, jt, jf, k }
        }
    }

    /// BPF program header (matches `struct sock_fprog`).
    #[repr(C)]
    struct SockFprog {
        len: libc::c_ushort,
        filter: *const SockFilter,
    }

    // Helper constructors for readability
    const fn bpf_stmt(code: u16, k: u32) -> SockFilter {
        SockFilter::new(code, 0, 0, k)
    }
    const fn bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter {
        SockFilter::new(code, jt, jf, k)
    }

    /// Low 16 bits of the return value encode errno.
    const fn seccomp_ret_errno(errno: u32) -> u32 {
        SECCOMP_RET_ERRNO | (errno & 0xFFFF)
    }

    /// Activate the seccomp-BPF sandbox.
    ///
    /// Returns `Ok(())` on success or an `io::Error` if seccomp installation fails.
    pub fn activate() -> io::Result<()> {
        // We must be on x86-64. Other arches would need different syscall numbers.
        #[cfg(not(target_arch = "x86_64"))]
        compile_error!("seccomp sandbox currently only supports x86-64");

        let eperm = seccomp_ret_errno(libc::EPERM as u32);

        // arg2 (prot) is at offset 16 + 2*8 = 32 in seccomp_data (low 32 bits)
        let arg2_lo_offset = SECCOMP_DATA_ARG_OFFSET + 2 * 8;

        // BPF filter program.
        //
        // The logic is:
        //   1. Verify architecture is x86-64 (kill otherwise)
        //   2. Load syscall number
        //   3. If execve or execveat or memfd_create → EPERM
        //   4. If mmap → check arg2 for PROT_EXEC → EPERM
        //   5. If mprotect → check arg2 for PROT_EXEC → EPERM
        //   6. Otherwise → ALLOW
        #[rustfmt::skip]
        let filter: &[SockFilter] = &[
            // [0] Load architecture
            bpf_stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_ARCH),
            // [1] Check x86-64; if not, kill
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
            // [2] Wrong arch → EPERM (safe fallback)
            bpf_stmt(BPF_RET | BPF_K, eperm),

            // [3] Load syscall number
            bpf_stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR),

            // [4] execve? → block
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, SYS_EXECVE, 0, 1),
            // [5] → EPERM
            bpf_stmt(BPF_RET | BPF_K, eperm),

            // [6] execveat? → block
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, SYS_EXECVEAT, 0, 1),
            // [7] → EPERM
            bpf_stmt(BPF_RET | BPF_K, eperm),

            // [8] memfd_create? → block
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, SYS_MEMFD_CREATE, 0, 1),
            // [9] → EPERM
            bpf_stmt(BPF_RET | BPF_K, eperm),

            // [10] mmap? → check PROT_EXEC in arg2
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, SYS_MMAP, 0, 4),
            // [11] Load prot argument (arg index 2, low 32 bits)
            bpf_stmt(BPF_LD | BPF_W | BPF_ABS, arg2_lo_offset),
            // [12] Mask with PROT_EXEC
            bpf_stmt(BPF_ALU | BPF_AND | BPF_K, PROT_EXEC),
            // [13] If result is non-zero (PROT_EXEC set) → block
            bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
            // [14] → EPERM
            bpf_stmt(BPF_RET | BPF_K, eperm),

            // [15] Allow everything else
            bpf_stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
        ];

        let prog = SockFprog {
            len: filter.len() as libc::c_ushort,
            filter: filter.as_ptr(),
        };

        // Allow ourselves to install a seccomp filter without being root.
        // PR_SET_NO_NEW_PRIVS is required before SECCOMP_SET_MODE_FILTER.
        let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        let ret = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                SECCOMP_SET_MODE_FILTER,
                0u32,
                &prog as *const SockFprog,
            )
        };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        tracing::info!("seccomp sandbox activated — execve, mmap(PROT_EXEC), memfd_create blocked (mprotect PROT_EXEC allowed for JIT)");
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod inner {
    use std::io;

    /// No-op on non-Linux platforms.
    pub fn activate() -> io::Result<()> {
        tracing::warn!("seccomp sandbox is only supported on Linux; skipping");
        Ok(())
    }
}

/// Activate the seccomp-BPF sandbox.
///
/// On Linux x86-64 this installs a BPF filter that blocks dangerous syscalls
/// (`execve`, `execveat`, `memfd_create`, `mmap` with `PROT_EXEC`).
///
/// On other platforms this is a no-op.
pub fn activate() -> std::io::Result<()> {
    inner::activate()
}

//! PPB_NetAddress_Private;1.1 / 1.0 / 0.1 implementation.
//!
//! Provides operations on opaque `PP_NetAddress_Private` structures.
//! Internally the `data` field stores a `sockaddr_in` (IPv4) or
//! `sockaddr_in6` (IPv6) in network byte order.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_void;
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

// ---------------------------------------------------------------------------
// Internal helpers: PP_NetAddress_Private ↔ SocketAddr conversion
// ---------------------------------------------------------------------------

/// Interpret a `PP_NetAddress_Private` as a Rust `SocketAddr`.
pub fn addr_to_socketaddr(addr: &PP_NetAddress_Private) -> Option<SocketAddr> {
    if addr.size == 0 {
        return None;
    }
    // We store sockaddr_in / sockaddr_in6 directly in `data`.
    let family = u16::from_ne_bytes([addr.data[0], addr.data[1]]);
    match family as i32 {
        libc::AF_INET => {
            if (addr.size as usize) < mem::size_of::<libc::sockaddr_in>() {
                return None;
            }
            let sa: libc::sockaddr_in =
                unsafe { std::ptr::read_unaligned(addr.data.as_ptr() as *const _) };
            let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
            let port = u16::from_be(sa.sin_port);
            Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        libc::AF_INET6 => {
            if (addr.size as usize) < mem::size_of::<libc::sockaddr_in6>() {
                return None;
            }
            let sa: libc::sockaddr_in6 =
                unsafe { std::ptr::read_unaligned(addr.data.as_ptr() as *const _) };
            let ip = Ipv6Addr::from(sa.sin6_addr.s6_addr);
            let port = u16::from_be(sa.sin6_port);
            let scope_id = sa.sin6_scope_id;
            Some(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope_id)))
        }
        _ => None,
    }
}

/// Write a `SocketAddr` into a `PP_NetAddress_Private`.
pub fn socketaddr_to_addr(sa: &SocketAddr, out: &mut PP_NetAddress_Private) {
    out.data = [0u8; 128];
    match sa {
        SocketAddr::V4(v4) => {
            let mut sin: libc::sockaddr_in = unsafe { mem::zeroed() };
            sin.sin_family = libc::AF_INET as u16;
            sin.sin_port = v4.port().to_be();
            sin.sin_addr.s_addr = u32::from(*v4.ip()).to_be();
            let size = mem::size_of::<libc::sockaddr_in>();
            out.size = size as u32;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &sin as *const _ as *const u8,
                    out.data.as_mut_ptr(),
                    size,
                );
            }
        }
        SocketAddr::V6(v6) => {
            let mut sin6: libc::sockaddr_in6 = unsafe { mem::zeroed() };
            sin6.sin6_family = libc::AF_INET6 as u16;
            sin6.sin6_port = v6.port().to_be();
            sin6.sin6_addr.s6_addr = v6.ip().octets();
            sin6.sin6_scope_id = v6.scope_id();
            let size = mem::size_of::<libc::sockaddr_in6>();
            out.size = size as u32;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &sin6 as *const _ as *const u8,
                    out.data.as_mut_ptr(),
                    size,
                );
            }
        }
    }
}

/// Get the address family from a PP_NetAddress_Private.
fn get_family_raw(addr: &PP_NetAddress_Private) -> i32 {
    if addr.size == 0 {
        return libc::AF_UNSPEC;
    }
    let family = u16::from_ne_bytes([addr.data[0], addr.data[1]]);
    family as i32
}

// ---------------------------------------------------------------------------
// Vtable functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn are_equal(
    addr1: *const PP_NetAddress_Private,
    addr2: *const PP_NetAddress_Private,
) -> PP_Bool {
    if addr1.is_null() || addr2.is_null() {
        return PP_FALSE;
    }
    let a1 = unsafe { &*addr1 };
    let a2 = unsafe { &*addr2 };
    let sa1 = addr_to_socketaddr(a1);
    let sa2 = addr_to_socketaddr(a2);
    pp_from_bool(sa1.is_some() && sa1 == sa2)
}

unsafe extern "C" fn are_hosts_equal(
    addr1: *const PP_NetAddress_Private,
    addr2: *const PP_NetAddress_Private,
) -> PP_Bool {
    if addr1.is_null() || addr2.is_null() {
        return PP_FALSE;
    }
    let a1 = unsafe { &*addr1 };
    let a2 = unsafe { &*addr2 };
    let sa1 = addr_to_socketaddr(a1);
    let sa2 = addr_to_socketaddr(a2);
    match (sa1, sa2) {
        (Some(s1), Some(s2)) => pp_from_bool(s1.ip() == s2.ip()),
        _ => PP_FALSE,
    }
}

unsafe extern "C" fn describe(
    _module: PP_Module,
    addr: *const PP_NetAddress_Private,
    include_port: PP_Bool,
) -> PP_Var {
    if addr.is_null() {
        return PP_Var::undefined();
    }
    let a = unsafe { &*addr };
    let Some(sa) = addr_to_socketaddr(a) else {
        return PP_Var::undefined();
    };

    let desc = if pp_to_bool(include_port) {
        match sa {
            SocketAddr::V4(v4) => format!("{}:{}", v4.ip(), v4.port()),
            SocketAddr::V6(v6) => format!("[{}]:{}", v6.ip(), v6.port()),
        }
    } else {
        sa.ip().to_string()
    };

    tracing::trace!("PPB_NetAddress_Private::Describe -> {:?}", desc);

    // Create a PP_Var string. Use the host's var manager.
    let Some(host) = crate::HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str(&desc)
}

unsafe extern "C" fn replace_port(
    src_addr: *const PP_NetAddress_Private,
    port: u16,
    addr_out: *mut PP_NetAddress_Private,
) -> PP_Bool {
    if src_addr.is_null() || addr_out.is_null() {
        return PP_FALSE;
    }
    let src = unsafe { &*src_addr };
    let Some(mut sa) = addr_to_socketaddr(src) else {
        return PP_FALSE;
    };
    sa.set_port(port);
    let out = unsafe { &mut *addr_out };
    socketaddr_to_addr(&sa, out);
    PP_TRUE
}

unsafe extern "C" fn get_any_address(is_ipv6: PP_Bool, addr: *mut PP_NetAddress_Private) {
    if addr.is_null() {
        return;
    }
    let out = unsafe { &mut *addr };
    if pp_to_bool(is_ipv6) {
        let sa = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        socketaddr_to_addr(&sa, out);
    } else {
        let sa = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
        socketaddr_to_addr(&sa, out);
    }
}

unsafe extern "C" fn get_family(
    addr: *const PP_NetAddress_Private,
) -> PP_NetAddressFamily_Private {
    if addr.is_null() {
        return PP_NETADDRESSFAMILY_PRIVATE_UNSPECIFIED;
    }
    let a = unsafe { &*addr };
    match get_family_raw(a) {
        f if f == libc::AF_INET => PP_NETADDRESSFAMILY_PRIVATE_IPV4,
        f if f == libc::AF_INET6 => PP_NETADDRESSFAMILY_PRIVATE_IPV6,
        _ => PP_NETADDRESSFAMILY_PRIVATE_UNSPECIFIED,
    }
}

unsafe extern "C" fn get_port(addr: *const PP_NetAddress_Private) -> u16 {
    if addr.is_null() {
        return 0;
    }
    let a = unsafe { &*addr };
    addr_to_socketaddr(a).map(|sa| sa.port()).unwrap_or(0)
}

unsafe extern "C" fn get_address(
    addr: *const PP_NetAddress_Private,
    address: *mut c_void,
    address_size: u16,
) -> PP_Bool {
    if addr.is_null() || address.is_null() {
        return PP_FALSE;
    }
    let a = unsafe { &*addr };
    let Some(sa) = addr_to_socketaddr(a) else {
        return PP_FALSE;
    };
    match sa {
        SocketAddr::V4(v4) => {
            let octets = v4.ip().octets();
            if (address_size as usize) < 4 {
                return PP_FALSE;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(octets.as_ptr(), address as *mut u8, 4);
            }
            PP_TRUE
        }
        SocketAddr::V6(v6) => {
            let octets = v6.ip().octets();
            if (address_size as usize) < 16 {
                return PP_FALSE;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(octets.as_ptr(), address as *mut u8, 16);
            }
            PP_TRUE
        }
    }
}

unsafe extern "C" fn get_scope_id(addr: *const PP_NetAddress_Private) -> u32 {
    if addr.is_null() {
        return 0;
    }
    let a = unsafe { &*addr };
    match addr_to_socketaddr(a) {
        Some(SocketAddr::V6(v6)) => v6.scope_id(),
        _ => 0,
    }
}

unsafe extern "C" fn create_from_ipv4_address(
    ip: *const u8,
    port: u16,
    addr_out: *mut PP_NetAddress_Private,
) {
    if ip.is_null() || addr_out.is_null() {
        return;
    }
    let octets: [u8; 4] = unsafe { std::ptr::read_unaligned(ip as *const [u8; 4]) };
    let ipv4 = Ipv4Addr::from(octets);
    let sa = SocketAddr::V4(SocketAddrV4::new(ipv4, port));
    let out = unsafe { &mut *addr_out };
    socketaddr_to_addr(&sa, out);
    tracing::trace!(
        "PPB_NetAddress_Private::CreateFromIPv4Address({}.{}.{}.{}:{})",
        octets[0], octets[1], octets[2], octets[3], port
    );
}

unsafe extern "C" fn create_from_ipv6_address(
    ip: *const u8,
    scope_id: u32,
    port: u16,
    addr_out: *mut PP_NetAddress_Private,
) {
    if ip.is_null() || addr_out.is_null() {
        return;
    }
    let octets: [u8; 16] = unsafe { std::ptr::read_unaligned(ip as *const [u8; 16]) };
    let ipv6 = Ipv6Addr::from(octets);
    let sa = SocketAddr::V6(SocketAddrV6::new(ipv6, port, 0, scope_id));
    let out = unsafe { &mut *addr_out };
    socketaddr_to_addr(&sa, out);
    tracing::trace!(
        "PPB_NetAddress_Private::CreateFromIPv6Address([{}]:{})",
        ipv6, port
    );
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_1_1: PPB_NetAddress_Private_1_1 = PPB_NetAddress_Private_1_1 {
    AreEqual: Some(are_equal),
    AreHostsEqual: Some(are_hosts_equal),
    Describe: Some(describe),
    ReplacePort: Some(replace_port),
    GetAnyAddress: Some(get_any_address),
    GetFamily: Some(get_family),
    GetPort: Some(get_port),
    GetAddress: Some(get_address),
    GetScopeID: Some(get_scope_id),
    CreateFromIPv4Address: Some(create_from_ipv4_address),
    CreateFromIPv6Address: Some(create_from_ipv6_address),
};

static VTABLE_1_0: PPB_NetAddress_Private_1_0 = PPB_NetAddress_Private_1_0 {
    AreEqual: Some(are_equal),
    AreHostsEqual: Some(are_hosts_equal),
    Describe: Some(describe),
    ReplacePort: Some(replace_port),
    GetAnyAddress: Some(get_any_address),
    GetFamily: Some(get_family),
    GetPort: Some(get_port),
    GetAddress: Some(get_address),
};

static VTABLE_0_1: PPB_NetAddress_Private_0_1 = PPB_NetAddress_Private_0_1 {
    AreEqual: Some(are_equal),
    AreHostsEqual: Some(are_hosts_equal),
    Describe: Some(describe),
    ReplacePort: Some(replace_port),
    GetAnyAddress: Some(get_any_address),
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_NETADDRESS_PRIVATE_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_NETADDRESS_PRIVATE_INTERFACE_1_0, &VTABLE_1_0);
        registry.register(PPB_NETADDRESS_PRIVATE_INTERFACE_0_1, &VTABLE_0_1);
    }
}

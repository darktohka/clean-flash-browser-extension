//! Minimal QOI (Quite OK Image) encoder.
//!
//! Encodes BGRA pixel data into the QOI format, converting BGRA → RGBA on
//! the fly.  The output is a standards-compliant QOI file that any QOI
//! decoder (including the hand-written WASM one in `qoiwasm/`) can read.
//!
//! Reference: <https://qoiformat.org/qoi-specification.pdf>

/// Encode a BGRA pixel buffer as a QOI image (RGBA, sRGB).
///
/// `pixels_bgra` must contain at least `width * height * 4` bytes in
/// BGRA channel order.  Returns the complete QOI file bytes.
pub fn qoi_encode_bgra(pixels_bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
    let px_count = (width as usize) * (height as usize);
    debug_assert!(
        pixels_bgra.len() >= px_count * 4,
        "pixel buffer too small for {}×{} image ({} < {})",
        width,
        height,
        pixels_bgra.len(),
        px_count * 4,
    );

    // Worst case: every pixel encoded as QOI_OP_RGBA (5 bytes each).
    let mut out = Vec::with_capacity(14 + px_count * 5 + 8);

    // ---- QOI header (14 bytes, big-endian) ----
    out.extend_from_slice(b"qoif");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.push(4); // channels = RGBA
    out.push(0); // colorspace = sRGB with linear alpha

    // ---- Encoding state ----
    let mut index = [[0u8; 4]; 64];
    let mut prev: [u8; 4] = [0, 0, 0, 255]; // RGBA
    let mut run: u16 = 0;

    for i in 0..px_count {
        let base = i * 4;
        // Convert BGRA → RGBA.
        let px = [
            pixels_bgra[base + 2], // R ← B offset in BGRA
            pixels_bgra[base + 1], // G
            pixels_bgra[base],     // B ← R offset in BGRA
            pixels_bgra[base + 3], // A
        ];

        if px == prev {
            run += 1;
            if run == 62 {
                out.push(0xC0 | (run as u8 - 1)); // QOI_OP_RUN
                run = 0;
            }
            continue;
        }

        // Flush any pending run before encoding a new pixel.
        if run > 0 {
            out.push(0xC0 | (run as u8 - 1)); // QOI_OP_RUN
            run = 0;
        }

        let hash = qoi_color_hash(&px);

        if index[hash] == px {
            // QOI_OP_INDEX (0b00_xxxxxx)
            out.push(hash as u8);
        } else {
            index[hash] = px;

            if px[3] == prev[3] {
                // Alpha unchanged — try diff / luma / rgb.
                let dr = px[0] as i16 - prev[0] as i16;
                let dg = px[1] as i16 - prev[1] as i16;
                let db = px[2] as i16 - prev[2] as i16;

                if dr >= -2 && dr <= 1 && dg >= -2 && dg <= 1 && db >= -2 && db <= 1 {
                    // QOI_OP_DIFF (0b01_dr_dg_db)
                    out.push(
                        0x40 | ((dr + 2) as u8) << 4
                            | ((dg + 2) as u8) << 2
                            | (db + 2) as u8,
                    );
                } else {
                    let dr_dg = dr - dg;
                    let db_dg = db - dg;
                    if dg >= -32
                        && dg <= 31
                        && dr_dg >= -8
                        && dr_dg <= 7
                        && db_dg >= -8
                        && db_dg <= 7
                    {
                        // QOI_OP_LUMA (0b10_dg) + (dr_dg | db_dg)
                        out.push(0x80 | (dg + 32) as u8);
                        out.push(((dr_dg + 8) as u8) << 4 | (db_dg + 8) as u8);
                    } else {
                        // QOI_OP_RGB (0xFE)
                        out.push(0xFE);
                        out.push(px[0]);
                        out.push(px[1]);
                        out.push(px[2]);
                    }
                }
            } else {
                // QOI_OP_RGBA (0xFF)
                out.push(0xFF);
                out.push(px[0]);
                out.push(px[1]);
                out.push(px[2]);
                out.push(px[3]);
            }
        }

        prev = px;
    }

    // Flush trailing run.
    if run > 0 {
        out.push(0xC0 | (run as u8 - 1));
    }

    // ---- QOI end marker (8 bytes) ----
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);

    out
}

/// QOI colour hash: `(r*3 + g*5 + b*7 + a*11) % 64`.
#[inline(always)]
fn qoi_color_hash(px: &[u8; 4]) -> usize {
    ((px[0] as usize)
        .wrapping_mul(3)
        .wrapping_add((px[1] as usize).wrapping_mul(5))
        .wrapping_add((px[2] as usize).wrapping_mul(7))
        .wrapping_add((px[3] as usize).wrapping_mul(11)))
        % 64
}

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

fn main() {
    let path_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/test_gps_photos/photo_0001.jpg".to_string());
    let path = Path::new(&path_str);
    println!("Testing EXIF parsing for: {:?}", path);

    // Read first 256KB
    let file = File::open(path).unwrap();
    let mut buf_reader = BufReader::with_capacity(256 * 1024, file);
    let mut header_buf = vec![0u8; 256 * 1024];
    let bytes_read = buf_reader.read(&mut header_buf).unwrap_or(0);
    header_buf.truncate(bytes_read);
    println!("Read {} bytes into buffer", bytes_read);
    println!("First 30 bytes: {:02x?}", &header_buf[..30.min(bytes_read)]);

    let exif_reader = exif::Reader::new();

    // Test 1: from buffer with Cursor
    println!("\n--- Test 1: read_from_container with Cursor ---");
    match exif_reader.read_from_container(&mut Cursor::new(&header_buf)) {
        Ok(exif) => {
            println!("SUCCESS! Found {} fields", exif.fields().count());
            for f in exif.fields() {
                println!(
                    "  {:?} ({:?}): {}",
                    f.tag,
                    f.ifd_num,
                    f.display_value().with_unit(&exif)
                );
            }
        }
        Err(e) => println!("FAILED: {}", e),
    }

    // Test 2: from file directly
    println!("\n--- Test 2: read_from_container with File ---");
    let file2 = File::open(path).unwrap();
    let mut reader2 = BufReader::new(file2);
    match exif_reader.read_from_container(&mut reader2) {
        Ok(exif) => {
            println!("SUCCESS! Found {} fields", exif.fields().count());
            for f in exif.fields() {
                println!(
                    "  {:?} ({:?}): {}",
                    f.tag,
                    f.ifd_num,
                    f.display_value().with_unit(&exif)
                );
            }
        }
        Err(e) => println!("FAILED: {}", e),
    }

    // Test 3: Try parsing just the EXIF segment directly
    println!("\n--- Test 3: Manual segment extraction ---");
    // Find APP1 marker
    let mut i = 2usize; // Skip SOI (FFD8)
    while i + 3 < bytes_read {
        if header_buf[i] == 0xFF {
            let marker = header_buf[i + 1];
            let length = u16::from_be_bytes([header_buf[i + 2], header_buf[i + 3]]) as usize;
            println!(
                "  Marker FF{:02X} at offset {}, length {}",
                marker, i, length
            );

            if marker == 0xE1 {
                // APP1 - should be EXIF
                let segment = &header_buf[i + 4..i + 2 + length];
                if segment.starts_with(b"Exif\0\0") {
                    let tiff_data = &segment[6..]; // Skip "Exif\0\0"
                    println!("  Found EXIF segment, TIFF data: {} bytes", tiff_data.len());
                    println!(
                        "  TIFF header: {:02x?}",
                        &tiff_data[..8.min(tiff_data.len())]
                    );

                    // Try parsing raw TIFF data directly
                    match exif_reader.read_raw(segment.to_vec()) {
                        Ok(exif) => {
                            println!("  read_raw SUCCESS! {} fields", exif.fields().count());
                            for f in exif.fields() {
                                println!(
                                    "    {:?} ({:?}): {}",
                                    f.tag,
                                    f.ifd_num,
                                    f.display_value().with_unit(&exif)
                                );
                            }
                        }
                        Err(e) => println!("  read_raw FAILED: {}", e),
                    }
                }
                break;
            }

            i += 2 + length;
        } else {
            i += 1;
        }
    }

    // Test 4: Use footos_core directly
    println!("\n--- Test 4: footos_core::read_metadata ---");
    match footos_core::read_metadata(path) {
        Ok(meta) => {
            println!("  lat: {:?}, lon: {:?}", meta.lat, meta.lon);
            println!("  date_taken: {:?}", meta.date_taken);
            println!("  make: {:?}, model: {:?}", meta.make, meta.model);
            println!("  width: {}, height: {}", meta.width, meta.height);
        }
        Err(e) => println!("  FAILED: {}", e),
    }
}

use fotos_core::read_metadata;
use std::path::Path;

fn main() {
    let path = Path::new("/Users/nice/Library/Developer/CoreSimulator/Devices/1A3B2904-7E39-4670-BEA0-05143A1AE60A/data/Media/DCIM/100APPLE/IMG_0006.HEIC");
    
    println!("Testing HEIC: {:?}", path);
    
    match read_metadata(path) {
        Ok(meta) => {
            println!("SUCCESS!");
            println!("  lat: {:?}", meta.lat);
            println!("  lon: {:?}", meta.lon);
            println!("  date: {:?}", meta.date_taken);
            println!("  make: {:?}", meta.make);
            println!("  model: {:?}", meta.model);
        }
        Err(e) => println!("ERROR: {:?}", e),
    }
}

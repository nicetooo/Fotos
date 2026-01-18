use std::path::Path;
use std::env;
use footos_core::read_metadata;

fn main() {
    let args: Vec<String> = env::args().collect();
    let path_str = args.get(1).map(|s| s.as_str()).unwrap_or("/tmp/test_photos/test_photo_001.jpg");
    let path = Path::new(path_str);
    println!("Testing: {:?}", path);
    
    match read_metadata(path) {
        Ok(meta) => {
            println!("Metadata read successfully:");
            println!("  lat: {:?}", meta.lat);
            println!("  lon: {:?}", meta.lon);
            println!("  date_taken: {:?}", meta.date_taken);
            println!("  width: {}", meta.width);
            println!("  height: {}", meta.height);
        }
        Err(e) => {
            println!("Error: {:?}", e);
        }
    }
}

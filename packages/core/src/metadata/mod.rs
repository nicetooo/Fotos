pub mod exif;
pub mod video;

pub use exif::{read_date_taken, read_metadata};
pub use video::read_video_metadata;

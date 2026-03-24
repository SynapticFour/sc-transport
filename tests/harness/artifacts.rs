use std::fs;
use std::path::PathBuf;

pub fn maybe_write_artifact(name: &str, body: &str) {
    if let Ok(dir) = std::env::var("SC_TRANSPORT_ARTIFACT_DIR") {
        let mut path = PathBuf::from(dir);
        let _ = fs::create_dir_all(&path);
        path.push(format!("{name}.json"));
        let _ = fs::write(path, body);
    }
}

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

fn regular(metadata: std::fs::Metadata) -> io::Result<()> {
    if metadata.is_file() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "not a regular file",
    ))
}

/// Reads at most `limit + 1` bytes, so callers can tell an oversized file from one at the limit.
pub fn read(path: &Path, limit: usize) -> io::Result<String> {
    // Checked before opening because opening a FIFO blocks, and after because the path can change.
    regular(path.metadata()?)?;
    let file = File::open(path)?;
    regular(file.metadata()?)?;
    let mut text = String::new();
    file.take((limit + 1) as u64).read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_byte_past_the_limit_and_rejects_directories_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spec.yaml");
        std::fs::write(&path, "abcdef").unwrap();
        assert_eq!(read(&path, 10).unwrap(), "abcdef");
        assert_eq!(read(&path, 3).unwrap(), "abcd");
        assert_eq!(
            read(dir.path(), 10).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            read(&dir.path().join("missing"), 10).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}

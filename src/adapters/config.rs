use crate::domain::variables::{CONFIG_LIMIT, Variables};
use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub fn path() -> Option<PathBuf> {
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let root = root.or_else(|| {
        if cfg!(windows) {
            return env::var_os("APPDATA").map(PathBuf::from);
        }
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
    })?;
    root.is_absolute()
        .then(|| root.join("viscurl").join("variables.json"))
}

pub fn load(path: &Path) -> Result<Variables, String> {
    match path.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err("Variables configuration must be a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Variables::default()),
        Err(error) => return Err(error.to_string()),
    }
    let mut text = String::new();
    File::open(path)
        .and_then(|file| {
            file.take((CONFIG_LIMIT + 1) as u64)
                .read_to_string(&mut text)
        })
        .map_err(|error| error.to_string())?;
    Variables::parse(&text)
}

pub fn save(path: &Path, variables: &Variables) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing configuration directory"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(variables.json().as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_reloads_variables_without_leaving_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("viscurl/variables.json");
        assert_eq!(load(&path).unwrap().json(), "{}");
        let vars = Variables::parse(r#"{"base_url":"http://localhost:8080"}"#).unwrap();
        save(&path, &vars).unwrap();
        assert_eq!(
            load(&path).unwrap().expand("{{base_url}}").unwrap(),
            "http://localhost:8080"
        );
        save(&path, &Variables::default()).unwrap();
        assert_eq!(load(&path).unwrap().json(), "{}");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}

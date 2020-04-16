use {
    directories::ProjectDirs,
    std::{
        ffi::OsString,
        fs,
        io::ErrorKind,
        path::{Path, PathBuf},
    },
};

const QUALIFIER: &str = "xyz.georgekaplan";
const ORG: &str = "g-s-k";
const APP_NAME: &str = "wear";
const DEFAULT_FILE_NAME: &str = "data.db";

pub(crate) fn database_file<P: AsRef<Path>>(
    user_path: Option<P>,
) -> anyhow::Result<(PathBuf, OsString)> {
    let mut directory;
    let mut file_name = OsString::from(DEFAULT_FILE_NAME);

    if let Some(p) = user_path {
        directory = p.as_ref().to_path_buf();

        match fs::metadata(&p) {
            // if the specified path exists and is a directory, use it with the default filename
            Ok(m) if m.is_dir() => (),

            // if it's a file, try splitting off the filename
            Ok(m) if m.is_file() => {
                if let (Some(d), Some(f)) = (directory.parent(), directory.file_name()) {
                    file_name = f.to_os_string();
                    directory = d.to_path_buf();
                }
            }

            // if it's something else, hmm...
            Ok(_) => (),

            // if it doesn't exist yet...
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // and it has a file extension, use it in its entirety
                if let (Some(d), Some(f), Some(_)) = (
                    directory.parent(),
                    directory.file_name(),
                    directory.extension(),
                ) {
                    file_name = f.to_os_string();
                    directory = d.to_path_buf();
                }
            }

            // otherwise, get the heck out of here
            Err(other) => return Err(other.into()),
        }
    } else if let Some(p_dirs) = ProjectDirs::from(QUALIFIER, ORG, APP_NAME) {
        directory = p_dirs.data_dir().to_path_buf();
    } else {
        eprintln!("Could not determine a platform-appropriate location for data storage. Using the current directory.");
        directory = std::env::current_dir()?;
    };

    Ok((directory, file_name))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn none() -> anyhow::Result<()> {
        let p_dirs = ProjectDirs::from(QUALIFIER, ORG, APP_NAME).unwrap();

        assert_eq!(
            database_file(None as Option<&str>)?,
            (p_dirs.data_dir().into(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn current_dir() -> anyhow::Result<()> {
        assert_eq!(
            database_file(Some("."))?,
            (".".into(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn parent_dir() -> anyhow::Result<()> {
        assert_eq!(
            database_file(Some(".."))?,
            ("..".into(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn existing_dir() -> anyhow::Result<()> {
        let tmp = std::env::temp_dir();

        assert_eq!(
            database_file(Some(&tmp))?,
            (tmp.into(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn existing_file() -> anyhow::Result<()> {
        let mut tmp = std::env::temp_dir();
        tmp.push(DEFAULT_FILE_NAME);
        fs::write(&tmp, b"")?;

        assert_eq!(
            database_file(Some(&tmp))?,
            (std::env::temp_dir(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn non_existing_dir() -> anyhow::Result<()> {
        // make a path representing a deeply nested location under /tmp that (almost)
        // definitely does not exist
        let mut tmp = std::env::temp_dir();
        tmp.push(
            "abcdefghijkl"
                .chars()
                .map(|c| c.to_string())
                .collect::<PathBuf>(),
        );

        assert_eq!(
            database_file(Some(&tmp))?,
            (tmp.into(), DEFAULT_FILE_NAME.into())
        );
        Ok(())
    }

    #[test]
    fn non_existing_file() -> anyhow::Result<()> {
        const F_NAME: &str = "zyxwvut.db";

        // make a path representing a deeply nested location under /tmp that (almost)
        // definitely does not exist
        let mut tmp = std::env::temp_dir();
        tmp.push(
            "abcdefghijkl"
                .chars()
                .map(|c| c.to_string())
                .collect::<PathBuf>(),
        );
        tmp.push(F_NAME);

        assert_eq!(
            database_file(Some(&tmp))?,
            (tmp.parent().unwrap().into(), F_NAME.into())
        );
        Ok(())
    }
}

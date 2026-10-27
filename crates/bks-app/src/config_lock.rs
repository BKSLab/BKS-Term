//! Cooperative, nonblocking lock for one configuration file across windows.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

pub(crate) struct ConfigLock {
    _file: File,
}

impl ConfigLock {
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            reject_reparse(parent)?;
            fs::create_dir_all(parent)?;
            reject_reparse(parent)?;
        }
        reject_reparse(path)?;
        let mut filename = path.as_os_str().to_os_string();
        filename.push(".lock");
        let lock_path = Path::new(&filename);
        reject_reparse(lock_path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(lock_path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(reparse_error());
        }
        Ok(Self { _file: file })
    }
}

fn reject_reparse(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 => {
            Err(reparse_error())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn reparse_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Файл настроек, его папка и блокировка не должны быть ссылками или reparse point.",
    )
}

use std::error::Error as StdError;
use std::fmt;
use std::io::Error as IOError;

use tokio_dl_stream_to_disk::error::Error as TDSTDError;
use tokio_dl_stream_to_disk::error::ErrorKind as TDSTDErrorKind;

#[derive(Debug)]
pub enum ErrorKind {
    FileExists,
    DirectoryExists,
    DirectoryMissing,
    InvalidApp,
    Authentication,
    TermsOfService,
    PermissionDenied,
    InvalidResponse,
    LoginRequired,
    IO(IOError),
    Str(String),
    Other(Box<dyn StdError + Send + Sync>),
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    pub fn new(k: ErrorKind) -> Error {
        Error { kind: k }
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

impl From<IOError> for Error {
    fn from(err: IOError) -> Error {
        Error {
            kind: ErrorKind::IO(err),
        }
    }
}

impl From<Box<dyn StdError + Send + Sync>> for Error {
    fn from(err: Box<dyn StdError + Send + Sync>) -> Error {
        Error {
            kind: ErrorKind::Other(err),
        }
    }
}

impl From<TDSTDError> for Error {
    fn from(err: TDSTDError) -> Error {
        match err.kind() {
            TDSTDErrorKind::FileExists => Error {
                kind: ErrorKind::FileExists,
            },
            TDSTDErrorKind::DirectoryMissing => Error {
                kind: ErrorKind::DirectoryMissing,
            },
            TDSTDErrorKind::PermissionDenied => Error {
                kind: ErrorKind::PermissionDenied,
            },
            TDSTDErrorKind::InvalidResponse => Error {
                kind: ErrorKind::InvalidResponse,
            },
            TDSTDErrorKind::IO(_) => {
                let err = err.into_inner_io().unwrap();
                Error {
                    kind: ErrorKind::IO(err),
                }
            }
            TDSTDErrorKind::Other(_) => {
                let err = err.into_inner_other().unwrap();
                Error {
                    kind: ErrorKind::Other(err),
                }
            }
        }
    }
}

impl From<&str> for Error {
    fn from(err: &str) -> Error {
        Error {
            kind: ErrorKind::Str(err.to_string()),
        }
    }
}

impl From<String> for Error {
    fn from(err: String) -> Error {
        Error {
            kind: ErrorKind::Str(err),
        }
    }
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.kind() {
            ErrorKind::FileExists => write!(f, "File already exists"),
            ErrorKind::InvalidApp => write!(f, "Invalid app response"),
            ErrorKind::DirectoryExists => write!(f, "Directory already exists"),
            ErrorKind::DirectoryMissing => {
                write!(f, "Destination path provided is not a valid directory")
            }
            ErrorKind::Authentication => write!(
                f,
                "Could not authenticate with Google. Please provide a new oAuth token."
            ),
            ErrorKind::TermsOfService => write!(
                f,
                "Must accept Google Play Terms of Service before proceeding."
            ),
            ErrorKind::PermissionDenied => write!(f, "Cannot create file: permission denied"),
            ErrorKind::InvalidResponse => write!(f, "Invalid response from the remote host"),
            ErrorKind::LoginRequired => write!(f, "Logging in is required for this action"),
            ErrorKind::IO(err) => err.fmt(f),
            ErrorKind::Str(err) => err.fmt(f),
            ErrorKind::Other(err) => err.fmt(f),
        }
    }
}

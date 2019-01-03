use actix::Message;

use libc::c_ushort;

use std::convert::TryFrom;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};
use serde_json;

use crate::IO;

impl Message for TerminadoMessage {
    type Result = ();
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum TerminadoMessage {
    Resize { rows: c_ushort, cols: c_ushort },
    Stdin(IO),
    Stdout(IO),
}

impl TerminadoMessage {
    pub fn from_json(json: &str) -> Result<Self, ()> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|_| {
            error!("Invalid Terminado message: Invalid JSON");
            ()
        })?;

        let list: &Vec<serde_json::Value> = value.as_array().ok_or_else(|| {
            error!("Invalid Terminado message: Needs to be an array!");
        })?;

        match list
            .first()
            .ok_or_else(|| {
                error!("Invalid Terminado message: Empty array!");
            })?
            .as_str()
            .ok_or_else(|| {
                error!("Invalid Terminado message: Type field not a string!");
            })? {
            "stdin" => {
                if list.len() != 2 {
                    error!(r#"Invalid Terminado message: "stdin" length != 2"#);
                    return Err(());
                }

                Ok(TerminadoMessage::Stdin(IO::from(
                    list[1].as_str().ok_or_else(|| {
                        error!(r#"Invalid Terminado message: "stdin" needs to be a String"#);
                    })?,
                )))
            }
            "stdout" => {
                if list.len() != 2 {
                    error!(r#"Invalid Terminado message: "stdout" length != 2"#);
                    return Err(());
                }

                Ok(TerminadoMessage::Stdout(IO::from(
                    list[1].as_str().ok_or_else(|| {
                        error!(r#"Invalid Terminado message: "stdout" needs to be a String"#);
                    })?,
                )))
            }
            "set_size" => {
                if list.len() != 3 {
                    error!(r#"Invalid Terminado message: "set_size" length != 2"#);
                    return Err(());
                }

                let rows: u16 = u16::try_from(list[1].as_u64().ok_or_else(|| {
                    error!(
                        r#"Invalid Terminado message: "set_size" element 1 needs to be an integer"#
                    );
                })?)
                .map_err(|_| {
                    error!(r#"Invalid Terminado message. "set_size" rows out of range."#);
                })?;

                let cols: u16 = u16::try_from(list[2].as_u64().ok_or_else(|| {
                    error!(
                        r#"Invalid Terminado message: "set_size" element 2 needs to be an integer"#
                    );
                })?)
                .map_err(|_| {
                    error!(r#"Invalid Terminado message. "set_size" cols out of range."#);
                })?;

                Ok(TerminadoMessage::Resize { rows, cols })
            }
            v => {
                error!("Invalid Terminado message: Unknown type {:?}", v);
                Err(())
            }
        }
    }
}

impl Serialize for TerminadoMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TerminadoMessage::Resize { rows, cols } => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("set_size")?;
                seq.serialize_element(rows)?;
                seq.serialize_element(cols)?;
                seq.end()
            }
            TerminadoMessage::Stdin(stdin) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("stdin")?;
                seq.serialize_element(&String::from_utf8_lossy(stdin.0.as_ref()))?;
                seq.end()
            }
            TerminadoMessage::Stdout(stdin) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("stdout")?;
                seq.serialize_element(&String::from_utf8_lossy(stdin.0.as_ref()))?;
                seq.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_resize() {
        let res = TerminadoMessage::Resize { rows: 25, cols: 80 };

        assert_eq!(
            serde_json::to_string(&res).unwrap(),
            r#"["set_size",25,80]"#
        );
    }

    #[test]
    fn test_serialize_stdin() {
        let res = TerminadoMessage::Stdin(IO::from("hello world"));

        assert_eq!(
            serde_json::to_string(&res).unwrap(),
            r#"["stdin","hello world"]"#
        );
    }

    #[test]
    fn test_serialize_stdout() {
        let res = TerminadoMessage::stdout(IO::from("hello world"));

        assert_eq!(
            serde_json::to_string(&res).unwrap(),
            r#"["stdout","hello world"]"#
        );
    }

    #[test]
    fn test_deserialize_resize() {
        let json = r#"["set_size", 25, 80]"#;

        let value = TerminadoMessage::from_json(json).expect("Could not parse TerminadoMessage");

        assert_eq!(value, TerminadoMessage::Resize { rows: 25, cols: 80 });
    }

    #[test]
    fn test_deserialize_stdin() {
        let json = r#"["stdin", "hello world"]"#;

        let value = TerminadoMessage::from_json(json).expect("Could not parse TerminadoMessage");

        assert_eq!(value, TerminadoMessage::Stdin("hello world".into()));
    }

    #[test]
    fn test_deserialize_stdout() {
        let json = r#"["stdout", "hello world"]"#;

        let value = TerminadoMessage::from_json(json).expect("Could not parse TerminadoMessage");

        assert_eq!(value, TerminadoMessage::stdout("hello world".into()));
    }
}

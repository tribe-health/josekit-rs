use serde_json::{json, Map, Value};
use std::io::Read;
use std::string::ToString;
use anyhow::bail;

use crate::error::JoseError;

/// Represents JWK object.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Jwk {
    key_operations: Option<Vec<String>>,
    x509_certificate_chain: Option<Vec<Vec<u8>>>,
    x509_certificate_sha1_thumbprint: Option<Vec<u8>>,
    x509_certificate_sha256_thumbprint: Option<Vec<u8>>,
    params: Map<String, Value>,
}

impl Jwk {
    pub fn new(key_type: &str) -> Self {
        let mut params = Map::new();
        params.insert("kty".to_string(), json!(key_type));
        Self {
            key_operations: None,
            x509_certificate_chain: None,
            x509_certificate_sha1_thumbprint: None,
            x509_certificate_sha256_thumbprint: None,
            params,
        }
    }

    pub fn from_map(map: Map<String, Value>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let mut key_operations = None;
            let mut x509_certificate_chain = None;
            let mut x509_certificate_sha1_thumbprint = None;
            let mut x509_certificate_sha256_thumbprint = None;
            for (key, value) in &map {
                match key.as_str() {
                    "jku" | "x5u" | "kid" | "typ" | "cty" => match value {
                        Value::String(_) => {},
                        _ => bail!("The JWK {} parameter must be a string.", key),
                    },
                    "key_ops" => key_operations = match value {
                        Value::Array(vals) => {
                            let mut vec = Vec::with_capacity(vals.len());
                            for val in vals {
                                match val {
                                    Value::String(val) => vec.push(val.to_string()),
                                    _ => bail!("An element of the JWK {} parameter must be a string.", key),
                                }
                            }
                            Some(vec)
                        },
                        _ => bail!("The JWT {} parameter must be a array.", key),
                    },
                    "x5c" => x509_certificate_chain = match value {
                        Value::Array(vals) => {
                            let mut vec = Vec::with_capacity(vals.len());
                            for val in vals {
                                match val {
                                    Value::String(val) => {
                                        let decoded = base64::decode_config(val, base64::URL_SAFE_NO_PAD)?;
                                        vec.push(decoded);
                                    },
                                    _ => bail!("An element of the JWK {} parameter must be a string.", key),
                                }
                            }
                            Some(vec)
                        },
                        _ => bail!("The JWK {} parameter must be a array.", key),
                    },
                    "x5t" => x509_certificate_sha1_thumbprint = match value {
                        Value::String(val) => Some(base64::decode_config(val, base64::URL_SAFE_NO_PAD)?),
                        _ => bail!("The JWK {} parameter must be a string.", key),
                    },
                    "x5t#S256" => x509_certificate_sha256_thumbprint = match value {
                        Value::String(val) => Some(base64::decode_config(val, base64::URL_SAFE_NO_PAD)?),
                        _ => bail!("The JWK {} parameter must be a string.", key),
                    },
                    _ => {}
                }
            }

            Ok(Self {
                key_operations,
                x509_certificate_chain,
                x509_certificate_sha1_thumbprint,
                x509_certificate_sha256_thumbprint,
                params: map,
            })
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    pub fn from_reader(input: &mut dyn Read) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let params: Map<String, Value> = serde_json::from_reader(input)?;
            Ok(Self::from_map(params)?)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    pub fn from_slice(input: impl AsRef<[u8]>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let params: Map<String, Value> = serde_json::from_slice(input.as_ref())?;
            Ok(Self::from_map(params)?)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    /// Set a value for a key type parameter (kty).
    ///
    /// # Arguments
    /// * `value` - A key type
    pub fn set_key_type(&mut self, value: String) {
        self.params.insert("kty".to_string(), json!(value));
    }

    /// Return a value for a key type parameter (kty).
    pub fn key_type(&self) -> &String {
        match self.params.get("kty") {
            Some(Value::String(val)) => val,
            _ => unreachable!("The JWS kty parameter is required."),
        }
    }

    /// Set a value for a key use parameter (use).
    ///
    /// # Arguments
    /// * `value` - A key use
    pub fn set_key_use(&mut self, value: Option<String>) {
        match value {
            Some(val) => {
                self.params.insert("use".to_string(), json!(val));
            },
            None => {
                self.params.remove("use");
            },
        }
    }

    /// Return a value for a key use parameter (use).
    pub fn key_use(&self) -> Option<&String> {
        match self.params.get("use") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set values for a key operations parameter (key_ops).
    ///
    /// # Arguments
    /// * `values` - key operations
    pub fn set_key_operations(&mut self, values: Option<Vec<String>>) {
        match &values {
            Some(vals) => {
                self.params.insert("key_ops".to_string(), json!(vals));
            },
            None => {
                self.params.remove("key_ops");
            },
        }
        self.key_operations = values;
    }

    /// Return values for a key operations parameter (key_ops).
    pub fn key_operations(&self) -> Option<&Vec<String>> {
        match self.key_operations {
            Some(ref val) => Some(val),
            None => None,
        }
    }

    /// Set a value for a algorithm parameter (alg).
    ///
    /// # Arguments
    /// * `value` - A algorithm
    pub fn set_algorithm(&mut self, value: Option<String>) {
        match value {
            Some(val) => {
                self.params.insert("alg".to_string(), json!(val));
            }
            None => {
                self.params.remove("alg");
            }
        }
    }

    /// Return a value for a algorithm parameter (alg).
    pub fn algorithm(&self) -> Option<&String> {
        match self.params.get("alg") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for a key ID parameter (kid).
    ///
    /// # Arguments
    /// * `value` - A key ID
    pub fn set_key_id(&mut self, value: Option<String>) {
        match value {
            Some(val) => {
                self.params.insert("kid".to_string(), json!(val));
            }
            None => {
                self.params.remove("kid");
            }
        }
    }

    /// Return a value for a key ID parameter (kid).
    pub fn key_id(&self) -> Option<&String> {
        match self.params.get("kid") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for a x509 url parameter (x5u).
    ///
    /// # Arguments
    /// * `value` - A x509 url
    pub fn set_x509_url(&mut self, value: Option<String>) {
        match value {
            Some(val) => {
                self.params.insert("x5u".to_string(), json!(val));
            }
            None => {
                self.params.remove("x5u");
            }
        }
    }

    /// Return a value for a x509 url parameter (x5u).
    pub fn x509_url(&self) -> Option<&String> {
        match self.params.get("x5u") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for a x509 certificate SHA-1 thumbprint parameter (x5t).
    ///
    /// # Arguments
    /// * `value` - A x509 certificate SHA-1 thumbprint
    pub fn set_x509_certificate_sha1_thumbprint(&mut self, value: Option<Vec<u8>>) {
        match &value {
            Some(val) => {
                self.params.insert("x5t".to_string(), Value::String(base64::encode_config(val, base64::URL_SAFE_NO_PAD)));
            }
            None => {
                self.params.remove("x5t");
            }
        }
        self.x509_certificate_sha1_thumbprint = value;
    }

    /// Return a value for a x509 certificate SHA-1 thumbprint parameter (x5t).
    pub fn x509_certificate_sha1_thumbprint(&self) -> Option<&Vec<u8>> {
        match self.x509_certificate_sha1_thumbprint {
            Some(ref val) => Some(val),
            None => None,
        }
    }

    /// Set a value for a x509 certificate SHA-256 thumbprint parameter (x5t#S256).
    ///
    /// # Arguments
    /// * `value` - A x509 certificate SHA-256 thumbprint
    pub fn set_x509_certificate_sha256_thumbprint(&mut self, value: Option<Vec<u8>>) {
        match &value {
            Some(val) => {
                self.params.insert("x5t#S256".to_string(), Value::String(base64::encode_config(val, base64::URL_SAFE_NO_PAD)));
            }
            None => {
                self.params.remove("x5t#S256");
            }
        }
        self.x509_certificate_sha256_thumbprint = value;
    }

    /// Return a value for a x509 certificate SHA-256 thumbprint parameter (x5t#S256).
    pub fn x509_certificate_sha256_thumbprint(&self) -> Option<&Vec<u8>> {
        match self.x509_certificate_sha256_thumbprint {
            Some(ref val) => Some(val),
            None => None,
        }
    }

    /// Set values for a X.509 certificate chain parameter (x5c).
    ///
    /// # Arguments
    /// * `values` - X.509 certificate chain
    pub fn set_x509_certificate_chain(&mut self, values: Option<Vec<Vec<u8>>>) {
        match &values {
            Some(vals) => {
                let mut vec = Vec::with_capacity(vals.len());
                for val in vals {
                    vec.push(Value::String(base64::encode_config(val, base64::URL_SAFE_NO_PAD)));
                }
                self.params.insert("x5c".to_string(), Value::Array(vec));
            }
            None => {
                self.params.remove("x5c");
            }
        }
        self.x509_certificate_chain = values;
    }

    /// Return values for a X.509 certificate chain parameter (x5c).
    pub fn x509_certificate_chain(&self) -> Option<&Vec<Vec<u8>>> {
        match self.x509_certificate_chain {
            Some(ref val) => Some(val),
            None => None,
        }
    }

    /// Set a value for a parameter of a specified key.
    ///
    /// # Arguments
    /// * `key` - A key name of a parameter
    /// * `value` - A typed value of a parameter
    pub fn set_parameter(&mut self, key: &str, value: Option<Value>) -> Result<(), JoseError> {
        (|| -> anyhow::Result<()> {
            match key {
                "kty" => match &value {
                    Some(Value::String(_)) => {}
                    _ => bail!("The JWK {} parameter must be a string.", key),
                },
                "use" | "alg" | "kid" | "x5u" => match &value {
                    Some(Value::String(_)) => {
                        self.params.insert(key.to_string(), value.unwrap());
                    },
                    None => {
                        self.params.remove(key);
                    },
                    _ => bail!("The JWK {} parameter must be a string.", key),
                },
                "key_ops" => match &value {
                    Some(Value::Array(vals)) => {
                        let mut vec = Vec::with_capacity(vals.len());
                        for val in vals {
                            match val {
                                Value::String(val) => vec.push(val.to_string()),
                                _ => bail!("An element of the JWT {} parameter must be a string.", key),
                            }
                        }
                        self.key_operations = Some(vec);
                        self.params.insert(key.to_string(), value.unwrap());
                    },
                    None => {
                        self.key_operations = None;
                        self.params.remove(key);
                    },
                    _ => bail!("The JWT {} parameter must be a array.", key),
                },
                "x5t" => match &value {
                    Some(Value::String(val)) => {
                        self.x509_certificate_sha1_thumbprint = Some(base64::decode_config(val, base64::URL_SAFE_NO_PAD)?);
                        self.params.insert(key.to_string(), value.unwrap());
                    },
                    None => {
                        self.x509_certificate_sha1_thumbprint = None;
                        self.params.remove(key);
                    },
                    _ => bail!("The JWK {} parameter must be a string.", key),
                },
                "x5t#S256" => match &value {
                    Some(Value::String(val)) => {
                        self.x509_certificate_sha256_thumbprint = Some(base64::decode_config(val, base64::URL_SAFE_NO_PAD)?);
                        self.params.insert(key.to_string(), value.unwrap());
                    },
                    None => {
                        self.x509_certificate_sha256_thumbprint = None;
                        self.params.remove(key);
                    },
                    _ => bail!("The JWK {} parameter must be a string.", key),
                },
                "x5c" => match &value {
                    Some(Value::Array(vals)) => {
                        let mut vec = Vec::with_capacity(vals.len());
                        for val in vals {
                            match val {
                                Value::String(val) => {
                                    let decoded = base64::decode_config(val, base64::URL_SAFE_NO_PAD)?;
                                    vec.push(decoded);
                                },
                                _ => bail!("An element of the JWK {} parameter must be a string.", key),
                            }
                        }
                        self.x509_certificate_chain = Some(vec);
                        self.params.insert(key.to_string(), value.unwrap());
                    },
                    None => {
                        self.x509_certificate_chain = None;
                        self.params.remove(key);
                    },
                    _ => bail!("The JWK {} parameter must be a string.", key),
                },
                _ => match &value {
                    Some(_) => {
                        self.params.insert(key.to_string(), value.unwrap());
                    }
                    None => {
                        self.params.remove(key);
                    }
                }
            }
            
            Ok(())
        })()
        .map_err(|err| JoseError::InvalidJwtFormat(err))
    }

    /// Return a value for a parameter of a specified key.
    ///
    /// # Arguments
    /// * `key` - A key name of a parameter
    pub fn parameter(&self, key: &str) -> Option<&Value> {
        self.params.get(key)
    }

    /// Return parameters
    pub fn parameters(&self) -> &Map<String, Value> {
        &self.params
    }
}

impl AsRef<Map<String, Value>> for Jwk {
    fn as_ref(&self) -> &Map<String, Value> {
        &self.params
    }
}

impl ToString for Jwk {
    fn to_string(&self) -> String {
        serde_json::to_string(&self.params).unwrap()
    }
}

/// Represents JWK set.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JwkSet {
    keys: Vec<Jwk>,
    params: Map<String, Value>,
}

impl JwkSet {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            params: Map::new(),
        }
    }

    pub fn from_map(map: Map<String, Value>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let mut map = map;
            let keys = match map.remove("keys") {
                Some(Value::Array(vals)) => {
                    let mut vec = Vec::new();
                    for val in vals {
                        match val {
                            Value::Object(val) => {
                                vec.push(Jwk::from_map(val)?);
                            },
                            _ => bail!("An element of the JWK set keys parameter must be a object."),
                        }
                    }
                    vec
                },
                Some(_) => bail!("The JWT keys parameter must be a array."),
                None => bail!("The JWK set must have a keys parameter."),
            };
            Ok(Self {
                keys,
                params: map,
            })
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    pub fn from_reader(input: &mut dyn Read) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let keys: Map<String, Value> = serde_json::from_reader(input)?;
            Ok(Self::from_map(keys)?)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    pub fn from_slice(input: impl AsRef<[u8]>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let keys: Map<String, Value> = serde_json::from_slice(input.as_ref())?;
            Ok(Self::from_map(keys)?)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err)
        })
    }

    pub fn keys(&self) -> &Vec<Jwk> {
        &self.keys
    }
}
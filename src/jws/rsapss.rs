use anyhow::bail;
use std::io::Read;
use openssl::hash::MessageDigest;
use openssl::pkey::{HasPublic, PKey, Private, Public};
use openssl::sign::{Signer, Verifier};
use serde_json::{Map, Value};
use once_cell::sync::Lazy;

use crate::jws::{JwsAlgorithm, JwsSigner, JwsVerifier};
use crate::jws::util::{json_eq, json_base64_bytes, parse_pem};
use crate::der::{DerReader, DerBuilder, DerType, DerClass};
use crate::der::oid::{ObjectIdentifier};
use crate::error::JoseError;

/// RSASSA-PSS using SHA-256 and MGF1 with SHA-256
pub const PS256: RsaPssJwsAlgorithm = RsaPssJwsAlgorithm::new("PS256");

/// RSASSA-PSS using SHA-384 and MGF1 with SHA-384
pub const PS384: RsaPssJwsAlgorithm = RsaPssJwsAlgorithm::new("PS384");

/// RSASSA-PSS using SHA-512 and MGF1 with SHA-512
pub const PS512: RsaPssJwsAlgorithm = RsaPssJwsAlgorithm::new("PS512");

static OID_RSASSA_PSS: Lazy<ObjectIdentifier> = Lazy::new(|| {
    ObjectIdentifier::from_slice(&[1, 2, 840, 113549, 1, 1, 10])
});

static OID_SHA256: Lazy<ObjectIdentifier> = Lazy::new(|| {
    ObjectIdentifier::from_slice(&[2, 16, 840, 1, 101, 3, 4, 2, 1])
});

static OID_SHA384: Lazy<ObjectIdentifier> = Lazy::new(|| {
    ObjectIdentifier::from_slice(&[2, 16, 840, 1, 101, 3, 4, 2, 2])
});

static OID_SHA512: Lazy<ObjectIdentifier> = Lazy::new(|| {
    ObjectIdentifier::from_slice(&[2, 16, 840, 1, 101, 3, 4, 2, 3])
});

static OID_MGF1: Lazy<ObjectIdentifier> = Lazy::new(|| {
    ObjectIdentifier::from_slice(&[1, 2, 840, 113549, 1, 1, 8])
});

#[derive(Debug, Eq, PartialEq)]
pub struct RsaPssJwsAlgorithm {
    name: &'static str,
}

impl RsaPssJwsAlgorithm {
    /// Return a new instance.
    ///
    /// # Arguments
    /// * `name` - A algrithm name.
    const fn new(name: &'static str) -> Self {
        RsaPssJwsAlgorithm {
            name,
        }
    }

    /// Return a signer from a private key of JWK format.
    ///
    /// # Arguments
    /// * `input` - A private key of JWK format.
    pub fn signer_from_jwk<'a>(
        &'a self,
        input: &[u8],
    ) -> Result<impl JwsSigner<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsSigner> {
            let map: Map<String, Value> = serde_json::from_slice(input)?;

            json_eq(&map, "alg", &self.name(), false)?;
            json_eq(&map, "kty", "RSA", true)?;
            json_eq(&map, "use", "sig", false)?;
            let n = json_base64_bytes(&map, "n")?;
            let e = json_base64_bytes(&map, "e")?;
            let d = json_base64_bytes(&map, "d")?;
            let p = json_base64_bytes(&map, "p")?;
            let q = json_base64_bytes(&map, "q")?;
            let dp = json_base64_bytes(&map, "dp")?;
            let dq = json_base64_bytes(&map, "dq")?;
            let qi = json_base64_bytes(&map, "qi")?;

            let mut builder = DerBuilder::new();
            builder.begin(DerType::Sequence);
            {
                builder.append_integer_from_u8(0); // version
                builder.append_integer_from_be_slice(&n); // n
                builder.append_integer_from_be_slice(&e); // e
                builder.append_integer_from_be_slice(&d); // d
                builder.append_integer_from_be_slice(&p); // p
                builder.append_integer_from_be_slice(&q); // q
                builder.append_integer_from_be_slice(&dp); // d mod (p-1)
                builder.append_integer_from_be_slice(&dq); // d mod (q-1)
                builder.append_integer_from_be_slice(&qi); // (inverse of q) mod p
            }
            builder.end();

            let pkcs8 = self.to_pkcs8(&builder.build(), false);
            let pkey = PKey::private_key_from_der(&pkcs8)?;
            self.check_key(&pkey)?;

            Ok(RsaPssJwsSigner {
                algorithm: &self,
                private_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Return a signer from a private key of PKCS#1 or PKCS#8 PEM format.
    ///
    /// # Arguments
    /// * `input` - A private key of PKCS#1 or PKCS#8 PEM format.
    pub fn signer_from_pem<'a>(
        &'a self,
        input: &[u8],
    ) -> Result<impl JwsSigner<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsSigner> {
            let (alg, data) = parse_pem(input)?;

            let pkey = match alg.as_str() {
                "PRIVATE KEY" | "RSA-PSS PRIVATE KEY" => {
                    if !self.detect_pkcs8(&data, false)? {
                        bail!("Invalid PEM contents.");
                    }
                    PKey::private_key_from_der(&data)?
                },
                "RSA PRIVATE KEY" => {
                    let pkcs8 = self.to_pkcs8(&data, false);
                    PKey::private_key_from_der(&pkcs8)?
                },
                alg => bail!("Inappropriate algorithm: {}", alg)
            };
            self.check_key(&pkey)?;

            Ok(RsaPssJwsSigner {
                algorithm: &self,
                private_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Return a signer from a private key of PKCS#1 or PKCS#8 DER format.
    ///
    /// # Arguments
    /// * `input` - A private key of PKCS#1 or PKCS#8 DER format.
    pub fn signer_from_der<'a>(
        &'a self,
        input: &'a [u8],
    ) -> Result<impl JwsSigner<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsSigner> {
            let pkey = if self.detect_pkcs8(input, false)? {
                PKey::private_key_from_der(input)?
            } else {
                let pkcs8 = self.to_pkcs8(input, false);
                PKey::private_key_from_der(&pkcs8)?
            };
            self.check_key(&pkey)?;

            Ok(RsaPssJwsSigner {
                algorithm: &self,
                private_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Return a verifier from a key of JWK format.
    ///
    /// # Arguments
    /// * `input` - A key of JWK format.
    pub fn verifier_from_jwk<'a>(
        &'a self,
        input: &[u8],
    ) -> Result<impl JwsVerifier<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsVerifier> {
            let map: Map<String, Value> = serde_json::from_slice(input)?;

            json_eq(&map, "alg", &self.name(), false)?;
            json_eq(&map, "kty", "RSA", true)?;
            json_eq(&map, "use", "sig", false)?;
            let n = json_base64_bytes(&map, "n")?;
            let e = json_base64_bytes(&map, "e")?;

            let mut builder = DerBuilder::new();
            builder.begin(DerType::Sequence);
            {
                builder.append_integer_from_be_slice(&n); // n
                builder.append_integer_from_be_slice(&e); // e
            }
            builder.end();
            
            let pkcs8 = self.to_pkcs8(&builder.build(), true);
            let pkey = PKey::public_key_from_der(&pkcs8)?;

            self.check_key(&pkey)?;

            Ok(RsaPssJwsVerifier {
                algorithm: &self,
                public_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Return a verifier from a public key of PKCS#1 or PKCS#8 PEM format.
    ///
    /// # Arguments
    /// * `input` - A public key of PKCS#1 or PKCS#8 PEM format.
    pub fn verifier_from_pem<'a>(
        &'a self,
        input: &[u8],
    ) -> Result<impl JwsVerifier<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsVerifier> {
            let (alg, data) = parse_pem(input)?;
            let pkey = match alg.as_str() {
                "PUBLIC KEY" | "RSA-PSS PUBLIC KEY" => {
                    if !self.detect_pkcs8(&data, true)? {
                        bail!("Invalid PEM contents.");
                    }
                    PKey::public_key_from_der(&data)?
                },
                "RSA PUBLIC KEY" => {
                    let pkcs8 = self.to_pkcs8(&data, true);
                    PKey::public_key_from_der(&pkcs8)?
                },
                alg => bail!("Inappropriate algorithm: {}", alg)
            };
            self.check_key(&pkey)?;

            Ok(RsaPssJwsVerifier {
                algorithm: &self,
                public_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Return a verifier from a public key of PKCS#1 or PKCS#8 DER format.
    ///
    /// # Arguments
    /// * `input` - A public key of PKCS#1 or PKCS#8 DER format.
    pub fn verifier_from_der<'a>(
        &'a self,
        input: &[u8],
    ) -> Result<impl JwsVerifier<Self> + 'a, JoseError> {
        (|| -> anyhow::Result<RsaPssJwsVerifier> {
            let pkey = if self.detect_pkcs8(input, true)? {
                PKey::public_key_from_der(input)?
            } else {
                let pkcs8 = self.to_pkcs8(input, true);
                PKey::public_key_from_der(&pkcs8)?
            };
            self.check_key(&pkey)?;

            Ok(RsaPssJwsVerifier {
                algorithm: &self,
                public_key: pkey,
            })
        })().map_err(|err| JoseError::InvalidKeyFormat(err))
    }
        
    fn parameters(&self) -> (&ObjectIdentifier, u8) {
        match self.name {
            "PS256" => (&OID_SHA256, 32),
            "PS384" => (&OID_SHA384, 48),
            "PS512" => (&OID_SHA512, 64),
            _ => unreachable!()
        }
    }

    fn check_key<T: HasPublic>(&self, pkey: &PKey<T>) -> anyhow::Result<()> {
        let rsa = pkey.rsa()?;

        if rsa.size() * 8 < 2048 {
            bail!("key length must be 2048 or more.");
        }

        Ok(())
    }
    
    fn detect_pkcs8(&self, input: &[u8], is_public:bool) -> anyhow::Result<bool> {
        let (sha_oid, salt_len) = self.parameters();

        let mut reader = DerReader::new(input.bytes());

        match reader.next() {
            Ok(Some(DerType::Sequence)) => {},
            _ => return Ok(false)
        }

        {
            if !is_public {
                // Version
                match reader.next() {
                    Ok(Some(DerType::Integer)) => {
                        match reader.to_u8() {
                            Ok(val) => {
                                if val != 0 {
                                    bail!("Unrecognized version: {}", val);
                                }
                            },
                            _ => return Ok(false)
                        }
                    },
                    _ => return Ok(false)
                }
            }

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {},
                _ => return Ok(false)
            }

            {
                match reader.next() {
                    Ok(Some(DerType::ObjectIdentifier)) => {
                        match reader.to_object_identifier() {
                            Ok(val) => {
                                if val != *OID_RSASSA_PSS {
                                    bail!("Incompatible oid: {}", val);
                                }
                            },
                            _ => return Ok(false)
                        }
                    },
                    _ => return Ok(false)
                }

                match reader.next() {
                    Ok(Some(DerType::Sequence)) => {},
                    _ => return Ok(false)
                }

                {
                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 0))) => {},
                        _ => return Ok(false)
                    }

                    match reader.next() {
                        Ok(Some(DerType::Sequence)) => {},
                        _ => return Ok(false)
                    }

                    {
                        match reader.next() {
                            Ok(Some(DerType::ObjectIdentifier)) => {
                                match reader.to_object_identifier() {
                                    Ok(val) => {
                                        if val != *sha_oid {
                                            bail!("Incompatible oid: {}", val);
                                        }
                                    },
                                    _ => return Ok(false)
                                }
                            },
                            _ => return Ok(false)
                        }
                    }

                    match reader.next() {
                        Ok(Some(DerType::EndOfContents)) => {},
                        _ => return Ok(false)
                    }

                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 1))) => {},
                        _ => return Ok(false)
                    }

                    match reader.next() {
                        Ok(Some(DerType::Sequence)) => {},
                        _ => return Ok(false)
                    }

                    {
                        match reader.next() {
                            Ok(Some(DerType::ObjectIdentifier)) => {
                                match reader.to_object_identifier() {
                                    Ok(val) => {
                                        if val != *OID_MGF1 {
                                            bail!("Incompatible oid: {}", val);
                                        }
                                    },
                                    _ => return Ok(false)
                                }
                            },
                            _ => return Ok(false)
                        }

                        match reader.next() {
                            Ok(Some(DerType::Sequence)) => {},
                            _ => return Ok(false)
                        }

                        {
                            match reader.next() {
                                Ok(Some(DerType::ObjectIdentifier)) => {
                                    match reader.to_object_identifier() {
                                        Ok(val) => {
                                            if val != *sha_oid {
                                                bail!("Incompatible oid: {}", val);
                                            }
                                        },
                                        _ => return Ok(false)
                                    }
                                },
                                _ => return Ok(false)
                            }
                        }
                    }

                    match reader.next() {
                        Ok(Some(DerType::EndOfContents)) => {},
                        _ => return Ok(false)
                    }

                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 2))) => {},
                        _ => return Ok(false)
                    }

                    match reader.next() {
                        Ok(Some(DerType::Integer)) => {
                            match reader.to_u8() {
                                Ok(val) => {
                                    if val != salt_len {
                                        bail!("Incompatible salt length: {}", val);
                                    }
                                },
                                _ => return Ok(false)
                            }
                        },
                        _ => return Ok(false)
                    }
                }
            }
        }

        Ok(true)
    }

    fn to_pkcs8(&self, input: &[u8], is_public: bool) -> Vec<u8> {
        let (sha_oid, salt_len) = self.parameters();

        let mut builder = DerBuilder::new();
        builder.begin(DerType::Sequence);
        {
            if !is_public {
                builder.append_integer_from_u8(0);
            }

            builder.begin(DerType::Sequence);
            {
                builder.append_object_identifier(&OID_RSASSA_PSS);
                builder.begin(DerType::Sequence);
                {
                    builder.begin(DerType::Other(DerClass::ContextSpecific, 0));
                    {
                        builder.begin(DerType::Sequence);
                        {
                            builder.append_object_identifier(sha_oid);
                        }
                        builder.end();
                    }
                    builder.end();

                    builder.begin(DerType::Other(DerClass::ContextSpecific, 1));
                    {
                        builder.begin(DerType::Sequence);
                        {
                            builder.append_object_identifier(&OID_MGF1);
                            builder.begin(DerType::Sequence);
                            {
                                builder.append_object_identifier(sha_oid);
                            }
                            builder.end();
                        }
                        builder.end();
                    }
                    builder.end();

                    builder.begin(DerType::Other(DerClass::ContextSpecific, 2));
                    {
                        builder.append_integer_from_u8(salt_len);
                    }
                    builder.end();
                }
                builder.end();
            }
            builder.end();
        }

        if is_public {
            builder.append_bit_string_from_slice(input, 0);
        } else {
            builder.append_octed_string_from_slice(input);
        }

        builder.end();
        builder.build()
    }
}

impl JwsAlgorithm for RsaPssJwsAlgorithm {
    fn name(&self) -> &str {
        self.name
    }
}

pub struct RsaPssJwsSigner<'a> {
    algorithm: &'a RsaPssJwsAlgorithm,
    private_key: PKey<Private>,
}

impl<'a> JwsSigner<RsaPssJwsAlgorithm> for RsaPssJwsSigner<'a> {
    fn algorithm(&self) -> &RsaPssJwsAlgorithm {
        &self.algorithm
    }

    fn sign(&self, input: &[&[u8]]) -> Result<Vec<u8>, JoseError> {
        (|| -> anyhow::Result<Vec<u8>> {
            let message_digest = match self.algorithm.name {
                "PS256" => MessageDigest::sha256(),
                "PS384" => MessageDigest::sha384(),
                "PS512" => MessageDigest::sha512(),
                _ => unreachable!(),
            };

            let mut signer = Signer::new(message_digest, &self.private_key)?;
            for part in input {
                signer.update(part)?;
            }
            let signature = signer.sign_to_vec()?;
            Ok(signature)
        })()
        .map_err(|err| JoseError::InvalidSignature(err))
    }
}

pub struct RsaPssJwsVerifier<'a> {
    algorithm: &'a RsaPssJwsAlgorithm,
    public_key: PKey<Public>,
}

impl<'a> JwsVerifier<RsaPssJwsAlgorithm> for RsaPssJwsVerifier<'a> {
    fn algorithm(&self) -> &RsaPssJwsAlgorithm {
        &self.algorithm
    }

    fn verify(&self, input: &[&[u8]], signature: &[u8]) -> Result<(), JoseError> {
        (|| -> anyhow::Result<()> {
            let message_digest = match self.algorithm.name {
                "PS256" => MessageDigest::sha256(),
                "PS384" => MessageDigest::sha384(),
                "PS512" => MessageDigest::sha512(),
                _ => unreachable!(),
            };

            let mut verifier = Verifier::new(message_digest, &self.public_key)?;
            for part in input {
                verifier.update(part)?;
            }
            verifier.verify(signature)?;
            Ok(())
        })()
        .map_err(|err| JoseError::InvalidSignature(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::Result;
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;

    #[test]
    fn sign_and_verify_rsspss_jwt() -> Result<()> {
        let data = b"abcde12345";

        for name in &[
            "PS256",
            "PS384",
            "PS512",
         ] {
            let alg = RsaPssJwsAlgorithm::new(name);

            let private_key = load_file("jwk/RSA_private.jwk")?;
            let public_key = load_file("jwk/RSA_public.jwk")?;

            let signer = alg.signer_from_jwk(&private_key)?;
            let signature = signer.sign(&[data])?;

            let verifier = alg.verifier_from_jwk(&public_key)?;
            verifier.verify(&[data], &signature)?;
        }

        Ok(())
    }

    #[test]
    fn sign_and_verify_rsspss_pkcs8_pem() -> Result<()> {
        let data = b"abcde12345";

        for name in &[
            "PS256",
            "PS384",
            "PS512",
         ] {
            let alg = RsaPssJwsAlgorithm::new(name);

            let private_key = load_file(match *name {
                "PS256" => "pem/rsapss_2048_sha256_pkcs8_private.pem",
                "PS384" => "pem/rsapss_2048_sha384_pkcs8_private.pem",
                "PS512" => "pem/rsapss_2048_sha512_pkcs8_private.pem",
                _ => unreachable!()
            })?;
            let public_key = load_file(match *name {
                "PS256" => "pem/rsapss_2048_sha256_pkcs8_public.pem",
                "PS384" => "pem/rsapss_2048_sha384_pkcs8_public.pem",
                "PS512" => "pem/rsapss_2048_sha512_pkcs8_public.pem",
                _ => unreachable!()
            })?;

            let signer = alg.signer_from_pem(&private_key)?;
            let signature = signer.sign(&[data])?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            verifier.verify(&[data], &signature)?;
        }

        Ok(())
    }

    #[test]
    fn sign_and_verify_rsspss_pkcs8_der() -> Result<()> {
        let data = b"abcde12345";

        for name in &[
            "PS256",
            "PS384",
            "PS512",
         ] {
            let alg = RsaPssJwsAlgorithm::new(name);

            let private_key = load_file(match *name {
                "PS256" => "der/rsapss_2048_sha256_pkcs8_private.der",
                "PS384" => "der/rsapss_2048_sha384_pkcs8_private.der",
                "PS512" => "der/rsapss_2048_sha512_pkcs8_private.der",
                _ => unreachable!()
            })?;
            let public_key = load_file(match *name {
                "PS256" => "der/rsapss_2048_sha256_pkcs8_public.der",
                "PS384" => "der/rsapss_2048_sha384_pkcs8_public.der",
                "PS512" => "der/rsapss_2048_sha512_pkcs8_public.der",
                _ => unreachable!()
            })?;

            let signer = alg.signer_from_der(&private_key)?;
            let signature = signer.sign(&[data])?;

            let verifier = alg.verifier_from_der(&public_key)?;
            verifier.verify(&[data], &signature)?;
        }

        Ok(())
    }

    fn load_file(path: &str) -> Result<Vec<u8>> {
        let mut pb = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        pb.push("data");
        pb.push(path);

        let mut file = File::open(&pb)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }
}
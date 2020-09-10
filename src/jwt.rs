mod payload;
mod payload_validator;

pub use crate::jwt::payload::JwtPayload;
pub use crate::jwt::payload_validator::JwtPayloadValidator;

use anyhow::bail;
use once_cell::sync::Lazy;
use serde_json::{Map, Value};

use crate::jose::{JoseError, JoseHeader};
use crate::jwe::{JweContext, JweDecrypter, JweEncrypter, JweHeader};
use crate::jwk::{Jwk, JwkSet};
use crate::jws::{JwsContext, JwsHeader, JwsSigner, JwsVerifier};
use crate::util;

static DEFAULT_CONTEXT: Lazy<JwtContext> = Lazy::new(|| JwtContext::new());

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JwtContext {
    jws_context: JwsContext,
    jwe_context: JweContext,
}

impl JwtContext {
    pub fn new() -> Self {
        Self {
            jws_context: JwsContext::new(),
            jwe_context: JweContext::new(),
        }
    }

    /// Test a critical header claim name is acceptable.
    ///
    /// # Arguments
    ///
    /// * `name` - a critical header claim name
    pub fn is_acceptable_critical(&self, name: &str) -> bool {
        self.jws_context.is_acceptable_critical(name)
    }

    /// Add a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn add_acceptable_critical(&mut self, name: &str) {
        self.jws_context.add_acceptable_critical(name);
        self.jwe_context.add_acceptable_critical(name);
    }

    /// Remove a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn remove_acceptable_critical(&mut self, name: &str) {
        self.jws_context.remove_acceptable_critical(name);
        self.jwe_context.remove_acceptable_critical(name);
    }

    /// Return the string repsentation of the JWT with a "none" algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWT heaser claims.
    pub fn encode_unsecured(
        &self,
        payload: &JwtPayload,
        header: &JwsHeader,
    ) -> Result<String, JoseError> {
        (|| -> anyhow::Result<String> {
            let mut header = header.claims_set().clone();
            header.insert("alg".to_string(), Value::String("none".to_string()));
            let header_bytes = serde_json::to_vec(&header)?;

            let payload_bytes = serde_json::to_vec(payload.claims_set())?;

            let mut capacity = 2;
            capacity += util::ceiling(header_bytes.len() * 4, 3);
            capacity += util::ceiling(payload_bytes.len() * 4, 3);

            let mut message = String::with_capacity(capacity);
            base64::encode_config_buf(header_bytes, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");
            base64::encode_config_buf(payload_bytes, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");

            Ok(message)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the string repsentation of the JWT with the siginig algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWS heaser claims.
    /// * `signer` - a signer object.
    pub fn encode_with_signer(
        &self,
        payload: &JwtPayload,
        header: &JwsHeader,
        signer: &dyn JwsSigner,
    ) -> Result<String, JoseError> {
        (|| -> anyhow::Result<String> {
            if let Some(vals) = header.critical() {
                if vals.iter().any(|val| val == "b64") {
                    bail!("JWT is not support b64 header claim.");
                }
            }

            let payload_bytes = serde_json::to_vec(payload.claims_set()).unwrap();
            let jwt = self
                .jws_context
                .serialize_compact(&payload_bytes, header, signer)?;
            Ok(jwt)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the string repsentation of the JWT with the encrypting algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWE heaser claims.
    /// * `encrypter` - a encrypter object.
    pub fn encode_with_encrypter(
        &self,
        payload: &JwtPayload,
        header: &JweHeader,
        encrypter: &dyn JweEncrypter,
    ) -> Result<String, JoseError> {
        let payload_bytes = serde_json::to_vec(payload.claims_set()).unwrap();
        let jwt = self
            .jwe_context
            .serialize_compact(&payload_bytes, header, encrypter)?;
        Ok(jwt)
    }

    /// Return the Jose header decoded from JWT.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    pub fn decode_header(&self, input: &str) -> Result<Box<dyn JoseHeader>, JoseError> {
        (|| -> anyhow::Result<Box<dyn JoseHeader>> {
            let parts: Vec<&str> = input.split('.').collect();
            if parts.len() == 3 {
                // JWS
                let header = base64::decode_config(parts[0], base64::URL_SAFE_NO_PAD)?;
                let header: Map<String, Value> = serde_json::from_slice(&header)?;
                let header = JwsHeader::from_map(header)?;
                Ok(Box::new(header))
            } else if parts.len() == 5 {
                // JWE
                let header = base64::decode_config(parts[0], base64::URL_SAFE_NO_PAD)?;
                let header: Map<String, Value> = serde_json::from_slice(&header)?;
                let header = JweHeader::from_map(header)?;
                Ok(Box::new(header))
            } else {
                bail!("The input cannot be recognized as a JWT.");
            }
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded with the "none" algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    pub fn decode_unsecured(&self, input: &str) -> Result<(JwtPayload, JwsHeader), JoseError> {
        (|| -> anyhow::Result<(JwtPayload, JwsHeader)> {
            let parts: Vec<&str> = input.split('.').collect();
            if parts.len() != 3 {
                bail!("The unsecured JWT must be three parts separated by colon.");
            }
            if parts[2].len() != 0 {
                bail!("The unsecured JWT must not have a signature part.");
            }

            let header = base64::decode_config(parts[0], base64::URL_SAFE_NO_PAD)?;
            let header: Map<String, Value> = serde_json::from_slice(&header)?;

            match header.get("alg") {
                Some(Value::String(val)) if val == "none" => {}
                Some(Value::String(val)) => bail!("The JWT alg header claim is not none: {}", val),
                Some(_) => bail!("The JWT alg header claim must be a string."),
                None => bail!("The JWT alg header claim is missing."),
            }

            match header.get("kid") {
                None => {}
                Some(_) => bail!("A JWT of none alg cannot have kid header claim."),
            }

            let header = JwsHeader::from_map(header)?;

            let payload = base64::decode_config(parts[1], base64::URL_SAFE_NO_PAD)?;
            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by the selected verifier.
    ///
    /// # Arguments
    ///
    /// * `verifier` - a verifier of the signing algorithm.
    /// * `input` - a JWT string representation.
    pub fn decode_with_verifier(
        &self,
        input: &str,
        verifier: &dyn JwsVerifier,
    ) -> Result<(JwtPayload, JwsHeader), JoseError> {
        self.decode_with_verifier_selector(input, |_header| Ok(Some(verifier)))
    }

    /// Return the JWT object decoded with a selected verifying algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `selector` - a function for selecting the verifying algorithm.
    pub fn decode_with_verifier_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(JwtPayload, JwsHeader), JoseError>
    where
        F: Fn(&JwsHeader) -> Result<Option<&'a dyn JwsVerifier>, JoseError>,
    {
        (|| -> anyhow::Result<(JwtPayload, JwsHeader)> {
            let (payload, header) =
                self.jws_context
                    .deserialize_compact_with_selector(input, |header| {
                        (|| -> anyhow::Result<Option<&'a dyn JwsVerifier>> {
                            let verifier = match selector(&header)? {
                                Some(val) => val,
                                None => return Ok(None),
                            };

                            if self.is_acceptable_critical("b64") {
                                bail!("JWT is not supported b64 header claim.");
                            }

                            Ok(Some(verifier))
                        })()
                        .map_err(|err| {
                            match err.downcast::<JoseError>() {
                                Ok(err) => err,
                                Err(err) => JoseError::InvalidJwtFormat(err),
                            }
                        })
                    })?;

            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by using a JWK set.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `jwk_set` - a JWK set.
    /// * `selector` - a function for selecting the verifying algorithm.
    pub fn decode_with_verifier_in_jwk_set<F>(
        &self,
        input: &str,
        jwk_set: &JwkSet,
        selector: F,
    ) -> Result<(JwtPayload, JwsHeader), JoseError>
    where
        F: Fn(&Jwk) -> Result<Option<&dyn JwsVerifier>, JoseError>,
    {
        self.decode_with_verifier_selector(input, |header| {
            let key_id = match header.key_id() {
                Some(val) => val,
                None => return Ok(None),
            };

            for jwk in jwk_set.get(key_id) {
                if let Some(val) = selector(jwk)? {
                    return Ok(Some(val));
                }
            }
            Ok(None)
        })
    }

    /// Return the JWT object decoded by the selected decrypter.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `decrypter` - a decrypter of the decrypting algorithm.
    pub fn decode_with_decrypter(
        &self,
        input: &str,
        decrypter: &dyn JweDecrypter,
    ) -> Result<(JwtPayload, JweHeader), JoseError> {
        self.decode_with_decrypter_selector(input, |_header| Ok(Some(decrypter)))
    }

    /// Return the JWT object decoded with a selected decrypting algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `decrypter_selector` - a function for selecting the decrypting algorithm.
    pub fn decode_with_decrypter_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(JwtPayload, JweHeader), JoseError>
    where
        F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
    {
        (|| -> anyhow::Result<(JwtPayload, JweHeader)> {
            let (payload, header) =
                self.jwe_context
                    .deserialize_compact_with_selector(input, |header| {
                        let decrypter = match selector(&header)? {
                            Some(val) => val,
                            None => return Ok(None),
                        };

                        Ok(Some(decrypter))
                    })?;

            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by using a JWK set.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `jwk_set` - a JWK set.
    /// * `selector` - a function for selecting the decrypting algorithm.
    pub fn decode_with_decrypter_in_jwk_set<F>(
        &self,
        input: &str,
        jwk_set: &JwkSet,
        selector: F,
    ) -> Result<(JwtPayload, JweHeader), JoseError>
    where
        F: Fn(&Jwk) -> Result<Option<&dyn JweDecrypter>, JoseError>,
    {
        self.decode_with_decrypter_selector(input, |header| {
            let key_id = match header.key_id() {
                Some(val) => val,
                None => return Ok(None),
            };

            for jwk in jwk_set.get(key_id) {
                if let Some(val) = selector(jwk)? {
                    return Ok(Some(val));
                }
            }
            Ok(None)
        })
    }
}

/// Return the string repsentation of the JWT with a "none" algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWT heaser claims.
pub fn encode_unsecured(payload: &JwtPayload, header: &JwsHeader) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_unsecured(payload, header)
}

/// Return the string repsentation of the JWT with the siginig algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWS heaser claims.
/// * `signer` - a signer object.
pub fn encode_with_signer(
    payload: &JwtPayload,
    header: &JwsHeader,
    signer: &dyn JwsSigner,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_with_signer(payload, header, signer)
}

/// Return the string repsentation of the JWT with the encrypting algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWE heaser claims.
/// * `encrypter` - a encrypter object.
pub fn encode_with_encrypter(
    payload: &JwtPayload,
    header: &JweHeader,
    encrypter: &dyn JweEncrypter,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_with_encrypter(payload, header, encrypter)
}

/// Return the Jose header decoded from JWT.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
pub fn decode_header(input: &str) -> Result<Box<dyn JoseHeader>, JoseError> {
    DEFAULT_CONTEXT.decode_header(input)
}

/// Return the JWT object decoded with the "none" algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
pub fn decode_unsecured(input: &str) -> Result<(JwtPayload, JwsHeader), JoseError> {
    DEFAULT_CONTEXT.decode_unsecured(input)
}

/// Return the JWT object decoded by the selected verifier.
///
/// # Arguments
///
/// * `verifier` - a verifier of the signing algorithm.
/// * `input` - a JWT string representation.
pub fn decode_with_verifier(
    input: &str,
    verifier: &dyn JwsVerifier,
) -> Result<(JwtPayload, JwsHeader), JoseError> {
    DEFAULT_CONTEXT.decode_with_verifier(input, verifier)
}

/// Return the JWT object decoded with a selected verifying algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `selector` - a function for selecting the verifying algorithm.
pub fn decode_with_verifier_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(JwtPayload, JwsHeader), JoseError>
where
    F: Fn(&JwsHeader) -> Result<Option<&'a dyn JwsVerifier>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_verifier_selector(input, selector)
}

/// Return the JWT object decoded by using a JWK set.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `jwk_set` - a JWK set.
/// * `selector` - a function for selecting the verifying algorithm.
pub fn decode_with_verifier_in_jwk_set<F>(
    input: &str,
    jwk_set: &JwkSet,
    selector: F,
) -> Result<(JwtPayload, JwsHeader), JoseError>
where
    F: Fn(&Jwk) -> Result<Option<&dyn JwsVerifier>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_verifier_in_jwk_set(input, jwk_set, selector)
}

/// Return the JWT object decoded by the selected decrypter.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `decrypter` - a decrypter of the decrypting algorithm.
pub fn decode_with_decrypter(
    input: &str,
    decrypter: &dyn JweDecrypter,
) -> Result<(JwtPayload, JweHeader), JoseError> {
    DEFAULT_CONTEXT.decode_with_decrypter(input, decrypter)
}

/// Return the JWT object decoded with a selected decrypting algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `decrypter_selector` - a function for selecting the decrypting algorithm.
pub fn decode_with_decrypter_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(JwtPayload, JweHeader), JoseError>
where
    F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_decrypter_selector(input, selector)
}

/// Return the JWT object decoded by using a JWK set.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `jwk_set` - a JWK set.
/// * `selector` - a function for selecting the decrypting algorithm.
pub fn decode_with_decrypter_in_jwk_set<F>(
    input: &str,
    jwk_set: &JwkSet,
    selector: F,
) -> Result<(JwtPayload, JweHeader), JoseError>
where
    F: Fn(&Jwk) -> Result<Option<&dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_decrypter_in_jwk_set(input, jwk_set, selector)
}

#[cfg(test)]
mod tests {
    #![allow(deprecated)]

    use anyhow::Result;
    use serde_json::json;
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use crate::jwe::{
        A128GcmKw, A128Kw, A192GcmKw, A192Kw, A256GcmKw, A256Kw, Dir, EcdhEs, EcdhEsA128Kw,
        EcdhEsA192Kw, EcdhEsA256Kw, Pbes2HS256A128Kw, Pbes2HS384A192Kw, Pbes2HS512A256Kw, Rsa1_5,
        RsaOaep,
    };
    use crate::jwk::Jwk;
    use crate::jws::{
        EdDSA, JwsHeader, ES256, ES256K, ES384, ES512, HS256, HS384, HS512, PS256, PS384, PS512,
        RS256, RS384, RS512,
    };
    use crate::jwt::{self, JwtPayload, JwtPayloadValidator};
    use crate::util;

    #[test]
    fn test_new_header() -> Result<()> {
        let mut header = JwsHeader::new();
        let jwk = Jwk::new("oct");
        header.set_jwk_set_url("jku");
        header.set_jwk(jwk.clone());
        header.set_x509_url("x5u");
        header.set_x509_certificate_chain(vec![b"x5c0".to_vec(), b"x5c1".to_vec()]);
        header.set_x509_certificate_sha1_thumbprint(b"x5t".to_vec());
        header.set_x509_certificate_sha256_thumbprint(b"x5t#S256".to_vec());
        header.set_key_id("kid");
        header.set_token_type("typ");
        header.set_content_type("cty");
        header.set_critical(vec!["crit0", "crit1"]);
        header.set_url("url");
        header.set_nonce(b"nonce".to_vec());
        header.set_claim("header_claim", Some(json!("header_claim")))?;

        assert!(matches!(header.jwk_set_url(), Some("jku")));
        assert!(matches!(header.jwk(), Some(val) if val == &jwk));
        assert!(matches!(header.x509_url(), Some("x5u")));
        assert!(
            matches!(header.x509_certificate_chain(), Some(vals) if vals == &vec![
                b"x5c0".to_vec(),
                b"x5c1".to_vec(),
            ])
        );
        assert!(
            matches!(header.x509_certificate_sha1_thumbprint(), Some(val) if val == &b"x5t".to_vec())
        );
        assert!(
            matches!(header.x509_certificate_sha256_thumbprint(), Some(val) if val == &b"x5t#S256".to_vec())
        );
        assert!(matches!(header.key_id(), Some("kid")));
        assert!(matches!(header.token_type(), Some("typ")));
        assert!(matches!(header.content_type(), Some("cty")));
        assert!(matches!(header.url(), Some("url")));
        assert!(matches!(header.nonce(), Some(val) if val == &b"nonce".to_vec()));
        assert!(matches!(header.critical(), Some(vals) if vals == &vec!["crit0", "crit1"]));
        assert!(matches!(header.claim("header_claim"), Some(val) if val == &json!("header_claim")));

        Ok(())
    }

    #[test]
    fn test_decode_header() -> Result<()> {
        let data = load_file("jwt/RS256.jwt")?;
        let data = String::from_utf8(data)?;
        let header = jwt::decode_header(&data)?;
        assert!(matches!(header.algorithm(), Some("RS256")));

        Ok(())
    }

    #[test]
    fn test_jwt_unsecured() -> Result<()> {
        let mut src_header = JwsHeader::new();
        src_header.set_token_type("JWT");
        let src_payload = JwtPayload::new();
        let jwt_string = jwt::encode_unsecured(&src_payload, &src_header)?;
        let (dst_payload, dst_header) = jwt::decode_unsecured(&jwt_string)?;

        src_header.set_claim("alg", Some(json!("none")))?;
        assert_eq!(src_header, dst_header);
        assert_eq!(src_payload, dst_payload);

        Ok(())
    }

    #[test]
    fn test_jwt_with_hmac() -> Result<()> {
        for alg in &[HS256, HS384, HS512] {
            let private_key = util::rand_bytes(64);

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_bytes(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_bytes(&private_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsa_pem() -> Result<()> {
        for alg in &[RS256, RS384, RS512] {
            let private_key = load_file("pem/RSA_2048bit_private.pem")?;
            let public_key = load_file("pem/RSA_2048bit_public.pem")?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsapss_pem() -> Result<()> {
        for alg in &[PS256, PS384, PS512] {
            let private_key = load_file(match alg.name() {
                "PS256" => "pem/RSA-PSS_2048bit_SHA-256_private.pem",
                "PS384" => "pem/RSA-PSS_2048bit_SHA-384_private.pem",
                "PS512" => "pem/RSA-PSS_2048bit_SHA-512_private.pem",
                _ => unreachable!(),
            })?;
            let public_key = load_file(match alg.name() {
                "PS256" => "pem/RSA-PSS_2048bit_SHA-256_public.pem",
                "PS384" => "pem/RSA-PSS_2048bit_SHA-384_public.pem",
                "PS512" => "pem/RSA-PSS_2048bit_SHA-512_public.pem",
                _ => unreachable!(),
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsa_der() -> Result<()> {
        for alg in &[RS256, RS384, RS512] {
            let private_key = load_file("der/RSA_2048bit_pkcs8_private.der")?;
            let public_key = load_file("der/RSA_2048bit_spki_public.der")?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_der(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_der(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_ecdsa_pem() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let private_key = load_file(match alg {
                ES256 => "pem/EC_P-256_private.pem",
                ES384 => "pem/EC_P-384_private.pem",
                ES512 => "pem/EC_P-521_private.pem",
                ES256K => "pem/EC_secp256k1_private.pem",
            })?;
            let public_key = load_file(match alg {
                ES256 => "pem/EC_P-256_public.pem",
                ES384 => "pem/EC_P-384_public.pem",
                ES512 => "pem/EC_P-521_public.pem",
                ES256K => "pem/EC_secp256k1_public.pem",
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_ecdsa_der() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let private_key = load_file(match alg {
                ES256 => "der/EC_P-256_pkcs8_private.der",
                ES384 => "der/EC_P-384_pkcs8_private.der",
                ES512 => "der/EC_P-521_pkcs8_private.der",
                ES256K => "der/EC_secp256k1_pkcs8_private.der",
            })?;
            let public_key = load_file(match alg {
                ES256 => "der/EC_P-256_spki_public.der",
                ES384 => "der/EC_P-384_spki_public.der",
                ES512 => "der/EC_P-521_spki_public.der",
                ES256K => "der/EC_secp256k1_spki_public.der",
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_der(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_der(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_payload_validate() -> Result<()> {
        let mut payload = JwtPayload::new();
        payload.set_issuer("iss");
        payload.set_subject("sub");
        payload.set_audience(vec!["aud0", "aud1"]);
        payload.set_expires_at(SystemTime::UNIX_EPOCH + Duration::from_secs(60));
        payload.set_not_before(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        payload.set_issued_at(SystemTime::UNIX_EPOCH);
        payload.set_jwt_id("jti");
        payload.set_claim("payload_claim", Some(json!("payload_claim")))?;

        let mut validator = JwtPayloadValidator::new();
        validator.set_base_time(SystemTime::UNIX_EPOCH + Duration::from_secs(30));
        validator.set_issuer("iss");
        validator.set_audience("aud1");
        validator.set_claim("payload_claim", json!("payload_claim"));
        validator.validate(&payload)?;

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_hmac() -> Result<()> {
        let jwk = Jwk::from_bytes(&load_file("jwk/oct_512bit_private.jwk")?)?;

        for alg in &[HS256, HS384, HS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_rsa() -> Result<()> {
        let jwk = Jwk::from_bytes(&load_file("jwk/RSA_public.jwk")?)?;

        for alg in &[RS256, RS384, RS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_rsapss() -> Result<()> {
        let jwk = Jwk::from_bytes(&load_file("jwk/RSA_public.jwk")?)?;

        for alg in &[PS256, PS384, PS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_ecdsa() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let jwk = Jwk::from_bytes(&load_file(match alg {
                ES256 => "jwk/EC_P-256_public.jwk",
                ES384 => "jwk/EC_P-384_public.jwk",
                ES512 => "jwk/EC_P-521_public.jwk",
                ES256K => "jwk/EC_secp256k1_public.jwk",
            })?)?;
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_eddsa() -> Result<()> {
        for alg in &[EdDSA] {
            let jwk = Jwk::from_bytes(&load_file(match alg {
                EdDSA => "jwk/OKP_Ed25519_public.jwk",
            })?)?;
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_dir() -> Result<()> {
        for alg in vec![Dir] {
            for enc in vec!["A128CBC-HS256", "A256GCM"] {
                // println!("{} {}", alg.name(), enc);

                let jwk = load_file(match enc {
                    "A128CBC-HS256" => "jwk/oct_256bit_private.jwk",
                    "A256GCM" => "jwk/oct_256bit_private.jwk",
                    _ => unreachable!(),
                })?;
                let jwk = Jwk::from_bytes(&jwk)?;
                let decrypter = alg.decrypter_from_jwk(&jwk)?;
                let jwt_string =
                    String::from_utf8(load_file(&format!("jwt/{}_{}.jwt", alg.name(), enc))?)?;
                let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                assert_eq!(header.content_encryption(), Some(enc));
                assert_eq!(payload.issuer(), Some("joe"));
                assert_eq!(
                    payload.expires_at(),
                    Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                );
                assert_eq!(
                    payload.claim("http://example.com/is_root"),
                    Some(&json!(true))
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_ecdh_es() -> Result<()> {
        for alg in vec![EcdhEs, EcdhEsA128Kw, EcdhEsA192Kw, EcdhEsA256Kw] {
            for curve in vec!["P-256", "P-384", "P-521", "X25519"] {
                for enc in vec!["A128CBC-HS256", "A256GCM"] {
                    // println!("{} {} {}", alg.name(), curve, enc);

                    let jwk = load_file(match curve {
                        "P-256" => "jwk/EC_P-256_private.jwk",
                        "P-384" => "jwk/EC_P-384_private.jwk",
                        "P-521" => "jwk/EC_P-521_private.jwk",
                        "X25519" => "jwk/OKP_X25519_private.jwk",
                        _ => unreachable!(),
                    })?;

                    let jwk = Jwk::from_bytes(&jwk)?;
                    let decrypter = alg.decrypter_from_jwk(&jwk)?;
                    let jwt_string = String::from_utf8(load_file(&format!(
                        "jwt/{}_{}_{}.jwt",
                        alg.name(),
                        curve,
                        enc
                    ))?)?;
                    let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                    assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                    assert_eq!(header.content_encryption(), Some(enc));
                    assert_eq!(payload.issuer(), Some("joe"));
                    assert_eq!(
                        payload.expires_at(),
                        Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                    );
                    assert_eq!(
                        payload.claim("http://example.com/is_root"),
                        Some(&json!(true))
                    );
                }
            }
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_aeskw() -> Result<()> {
        for alg in vec![A128Kw, A192Kw, A256Kw] {
            for enc in vec!["A128CBC-HS256", "A256GCM"] {
                println!("{} {}", alg.name(), enc);

                let jwk = load_file(match alg {
                    A128Kw => "jwk/oct_128bit_private.jwk",
                    A192Kw => "jwk/oct_192bit_private.jwk",
                    A256Kw => "jwk/oct_256bit_private.jwk",
                })?;
                let jwk = Jwk::from_bytes(&jwk)?;
                let decrypter = alg.decrypter_from_jwk(&jwk)?;
                let jwt_string =
                    String::from_utf8(load_file(&format!("jwt/{}_{}.jwt", alg.name(), enc))?)?;
                let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                assert_eq!(header.content_encryption(), Some(enc));
                assert_eq!(payload.issuer(), Some("joe"));
                assert_eq!(
                    payload.expires_at(),
                    Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                );
                assert_eq!(
                    payload.claim("http://example.com/is_root"),
                    Some(&json!(true))
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_aesgcmkw() -> Result<()> {
        for alg in vec![A128GcmKw, A192GcmKw, A256GcmKw] {
            for enc in vec!["A128CBC-HS256", "A256GCM"] {
                println!("{} {}", alg.name(), enc);

                let jwk = load_file(match alg {
                    A128GcmKw => "jwk/oct_128bit_private.jwk",
                    A192GcmKw => "jwk/oct_192bit_private.jwk",
                    A256GcmKw => "jwk/oct_256bit_private.jwk",
                })?;
                let jwk = Jwk::from_bytes(&jwk)?;
                let decrypter = alg.decrypter_from_jwk(&jwk)?;
                let jwt_string =
                    String::from_utf8(load_file(&format!("jwt/{}_{}.jwt", alg.name(), enc))?)?;
                let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                assert_eq!(header.content_encryption(), Some(enc));
                assert_eq!(payload.issuer(), Some("joe"));
                assert_eq!(
                    payload.expires_at(),
                    Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                );
                assert_eq!(
                    payload.claim("http://example.com/is_root"),
                    Some(&json!(true))
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_pbes2_hmac_aeskw() -> Result<()> {
        for alg in vec![Pbes2HS256A128Kw, Pbes2HS384A192Kw, Pbes2HS512A256Kw] {
            for enc in vec!["A128CBC-HS256", "A256GCM"] {
                println!("{} {}", alg.name(), enc);

                let jwk = load_file(match alg {
                    Pbes2HS256A128Kw => "jwk/oct_128bit_private.jwk",
                    Pbes2HS384A192Kw => "jwk/oct_128bit_private.jwk",
                    Pbes2HS512A256Kw => "jwk/oct_128bit_private.jwk",
                })?;
                let jwk = Jwk::from_bytes(&jwk)?;
                let decrypter = alg.decrypter_from_jwk(&jwk)?;
                let jwt_string =
                    String::from_utf8(load_file(&format!("jwt/{}_{}.jwt", alg.name(), enc))?)?;
                let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                assert_eq!(header.content_encryption(), Some(enc));
                assert_eq!(payload.issuer(), Some("joe"));
                assert_eq!(
                    payload.expires_at(),
                    Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                );
                assert_eq!(
                    payload.claim("http://example.com/is_root"),
                    Some(&json!(true))
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_decrypt_with_rsaes() -> Result<()> {
        for alg in vec![Rsa1_5, RsaOaep] {
            for enc in vec!["A128CBC-HS256", "A256GCM"] {
                println!("{} {}", alg.name(), enc);

                let jwk = load_file("jwk/RSA_private.jwk")?;

                let jwk = Jwk::from_bytes(&jwk)?;
                let decrypter = alg.decrypter_from_jwk(&jwk)?;
                let jwt_string =
                    String::from_utf8(load_file(&format!("jwt/{}_{}.jwt", alg.name(), enc))?)?;
                let (payload, header) = jwt::decode_with_decrypter(&jwt_string, &decrypter)?;

                assert_eq!(header.algorithm(), Some(decrypter.algorithm().name()));
                assert_eq!(header.content_encryption(), Some(enc));
                assert_eq!(payload.issuer(), Some("joe"));
                assert_eq!(
                    payload.expires_at(),
                    Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
                );
                assert_eq!(
                    payload.claim("http://example.com/is_root"),
                    Some(&json!(true))
                );
            }
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

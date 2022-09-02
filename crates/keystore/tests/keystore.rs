// Copyright 2022 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use mas_jose::{
    jwk::ParametersInfo,
    jwt::{JsonWebSignatureHeader, Jwt},
};
use mas_keystore::PrivateKey;

static PASSWORD: &str = "hunter2";

/// Generate a test which loads a key, and then tries signing and verifying a
/// JWT for each available algorithm
macro_rules! plain_test {
    ($name:ident, $kind:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!(concat!("./keys/", $path));
            let key = PrivateKey::load(bytes).unwrap();
            assert!(matches!(key, PrivateKey::$kind(_)), "wrong key type");

            let algs = key.possible_algs();
            assert_ne!(algs.len(), 0);

            for &alg in algs {
                let header = JsonWebSignatureHeader::new(alg);
                let payload = "hello";
                let signer = key.signing_key_for_alg(alg).unwrap();
                let jwt = Jwt::sign(header, payload, &signer).unwrap();
                let verifier = key.verifying_key_for_alg(alg).unwrap();
                jwt.verify(&verifier).unwrap();
            }
        }
    };
}

/// Generate a test which loads an encrypted key, and then tries signing and
/// verifying a JWT for each available algorithm
macro_rules! enc_test {
    ($name:ident, $kind:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!(concat!("./keys/", $path));
            let key = PrivateKey::load_encrypted(bytes, PASSWORD).unwrap();
            assert!(matches!(key, PrivateKey::$kind(_)), "wrong key type");

            let algs = key.possible_algs();
            assert_ne!(algs.len(), 0);

            for &alg in algs {
                let header = JsonWebSignatureHeader::new(alg);
                let payload = "hello";
                let signer = key.signing_key_for_alg(alg).unwrap();
                let jwt = Jwt::sign(header, payload, &signer).unwrap();
                let verifier = key.verifying_key_for_alg(alg).unwrap();
                jwt.verify(&verifier).unwrap();
            }
        }
    };
}

/// Generate a PEM decoding and encoding test
macro_rules! pem_test {
    ($name:ident, $path:literal) => {
        pem_test!($name, $path, compare = true);
    };
    ($name:ident, $path:literal, compare = $compare:literal) => {
        #[test]
        fn $name() {
            let pem = include_str!(concat!("./keys/", $path, ".pem"));
            let key = PrivateKey::load_pem(pem).unwrap();
            let pem2 = key.to_pem(pem_rfc7468::LineEnding::LF).unwrap();

            if $compare {
                assert_eq!(pem, pem2.as_str());
            }
        }
    };
}

/// Generate a DER decoding and encoding test
macro_rules! der_test {
    ($name:ident, $path:literal) => {
        der_test!($name, $path, compare = true);
    };
    ($name:ident, $path:literal, compare = $compare:literal) => {
        #[test]
        fn $name() {
            let der = include_bytes!(concat!("./keys/", $path, ".der"));
            let key = PrivateKey::load_der(der).unwrap();
            let der2 = key.to_der().unwrap();

            if $compare {
                assert_eq!(der, der2.as_slice());
            }
        }
    };
}

plain_test!(plain_rsa_pkcs1_pem, Rsa, "rsa.pkcs1.pem");
plain_test!(plain_rsa_pkcs1_der, Rsa, "rsa.pkcs1.der");
plain_test!(plain_rsa_pkcs8_pem, Rsa, "rsa.pkcs8.pem");
plain_test!(plain_rsa_pkcs8_der, Rsa, "rsa.pkcs8.der");
plain_test!(plain_ec_p256_sec1_pem, EcP256, "ec-p256.sec1.pem");
plain_test!(plain_ec_p256_sec1_der, EcP256, "ec-p256.sec1.der");
plain_test!(plain_ec_p256_pkcs8_pem, EcP256, "ec-p256.pkcs8.pem");
plain_test!(plain_ec_p256_pkcs8_der, EcP256, "ec-p256.pkcs8.der");
plain_test!(plain_ec_p384_sec1_pem, EcP384, "ec-p384.sec1.pem");
plain_test!(plain_ec_p384_sec1_der, EcP384, "ec-p384.sec1.der");
plain_test!(plain_ec_p384_pkcs8_pem, EcP384, "ec-p384.pkcs8.pem");
plain_test!(plain_ec_p384_pkcs8_der, EcP384, "ec-p384.pkcs8.der");
plain_test!(plain_ec_k256_sec1_pem, EcK256, "ec-k256.sec1.pem");
plain_test!(plain_ec_k256_sec1_der, EcK256, "ec-k256.sec1.der");
plain_test!(plain_ec_k256_pkcs8_pem, EcK256, "ec-k256.pkcs8.pem");
plain_test!(plain_ec_k256_pkcs8_der, EcK256, "ec-k256.pkcs8.der");

enc_test!(enc_rsa_pkcs8_pem, Rsa, "rsa.pkcs8.encrypted.pem");
enc_test!(enc_rsa_pkcs8_der, Rsa, "rsa.pkcs8.encrypted.der");
enc_test!(enc_ec_p256_pkcs8_pem, EcP256, "ec-p256.pkcs8.encrypted.pem");
enc_test!(enc_ec_p256_pkcs8_der, EcP256, "ec-p256.pkcs8.encrypted.der");
enc_test!(enc_ec_p384_pkcs8_pem, EcP384, "ec-p384.pkcs8.encrypted.pem");
enc_test!(enc_ec_p384_pkcs8_der, EcP384, "ec-p384.pkcs8.encrypted.der");
enc_test!(enc_ec_k256_pkcs8_pem, EcK256, "ec-k256.pkcs8.encrypted.pem");
enc_test!(enc_ec_k256_pkcs8_der, EcK256, "ec-k256.pkcs8.encrypted.der");

// Test PEM/DER serialization. For some reason, DER serialization of EC keys
// have some extra bytes
pem_test!(serialize_rsa_pkcs1_pem, "rsa.pkcs1");
der_test!(serialize_rsa_pkcs1_der, "rsa.pkcs1");
pem_test!(serialize_ec_p256_sec1_pem, "ec-p256.sec1");
der_test!(serialize_ec_p256_sec1_der, "ec-p256.sec1", compare = false);
pem_test!(serialize_ec_p384_sec1_pem, "ec-p384.sec1");
der_test!(serialize_ec_p384_sec1_der, "ec-p384.sec1", compare = false);
pem_test!(serialize_ec_k256_sec1_pem, "ec-k256.sec1");
der_test!(serialize_ec_k256_sec1_der, "ec-k256.sec1", compare = false);

#[test]
fn load_encrypted_as_unencrypted_error() {
    let pem = include_str!("./keys/rsa.pkcs8.encrypted.pem");
    assert!(PrivateKey::load_pem(pem).unwrap_err().is_encrypted());

    let der = include_bytes!("./keys/rsa.pkcs8.encrypted.der");
    assert!(PrivateKey::load_der(der).unwrap_err().is_encrypted());
}

#[test]
fn load_unencrypted_as_encrypted_error() {
    let pem = include_str!("./keys/rsa.pkcs8.pem");
    assert!(PrivateKey::load_encrypted_pem(pem, PASSWORD)
        .unwrap_err()
        .is_unencrypted());

    let der = include_bytes!("./keys/rsa.pkcs8.der");
    assert!(PrivateKey::load_encrypted_der(der, PASSWORD)
        .unwrap_err()
        .is_unencrypted());
}
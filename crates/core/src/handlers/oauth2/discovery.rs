// Copyright 2021 The Matrix.org Foundation C.I.C.
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

use std::collections::HashSet;

use hyper::Method;
use mas_config::OAuth2Config;
use oauth2_types::{
    oidc::{Metadata, SigningAlgorithm},
    pkce::CodeChallengeMethod,
    requests::{ClientAuthenticationMethod, GrantType, ResponseMode},
    scope::{ADDRESS, EMAIL, OPENID, PHONE, PROFILE},
};
use warp::{Filter, Rejection, Reply};

use crate::filters::cors::cors;

pub(super) fn filter(
    config: &OAuth2Config,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone + Send + Sync + 'static {
    let base = config.issuer.clone();

    let response_modes_supported = Some({
        let mut s = HashSet::new();
        s.insert(ResponseMode::FormPost);
        s.insert(ResponseMode::Query);
        s.insert(ResponseMode::Fragment);
        s
    });

    let response_types_supported = Some({
        let mut s = HashSet::new();
        s.insert("code".to_string());
        s.insert("token".to_string());
        s.insert("id_token".to_string());
        s.insert("code token".to_string());
        s.insert("code id_token".to_string());
        s.insert("token id_token".to_string());
        s.insert("code token id_token".to_string());
        s
    });

    let grant_types_supported = Some({
        let mut s = HashSet::new();
        s.insert(GrantType::AuthorizationCode);
        s.insert(GrantType::RefreshToken);
        s
    });

    let token_endpoint_auth_methods_supported = Some({
        let mut s = HashSet::new();
        s.insert(ClientAuthenticationMethod::ClientSecretBasic);
        s.insert(ClientAuthenticationMethod::ClientSecretPost);
        s.insert(ClientAuthenticationMethod::ClientSecretJwt);
        s.insert(ClientAuthenticationMethod::None);
        s
    });

    let token_endpoint_auth_signing_alg_values_supported = Some({
        let mut s = HashSet::new();
        s.insert(SigningAlgorithm::Hs256);
        s.insert(SigningAlgorithm::Hs384);
        s.insert(SigningAlgorithm::Hs512);
        s
    });

    let code_challenge_methods_supported = Some({
        let mut s = HashSet::new();
        s.insert(CodeChallengeMethod::Plain);
        s.insert(CodeChallengeMethod::S256);
        s
    });

    let scopes_supported = Some(
        [OPENID, PROFILE, EMAIL, ADDRESS, PHONE]
            .iter()
            .map(ToString::to_string)
            .collect(),
    );

    let metadata = Metadata {
        authorization_endpoint: base.join("oauth2/authorize").ok(),
        token_endpoint: base.join("oauth2/token").ok(),
        jwks_uri: base.join("oauth2/keys.json").ok(),
        introspection_endpoint: base.join("oauth2/introspect").ok(),
        userinfo_endpoint: base.join("oauth2/userinfo").ok(),
        issuer: base,
        registration_endpoint: None,
        scopes_supported,
        response_types_supported,
        response_modes_supported,
        grant_types_supported,
        token_endpoint_auth_methods_supported,
        token_endpoint_auth_signing_alg_values_supported,
        code_challenge_methods_supported,
    };

    warp::path!(".well-known" / "openid-configuration").and(
        warp::get()
            .map(move || warp::reply::json(&metadata))
            .with(cors().allow_method(Method::GET)),
    )
}

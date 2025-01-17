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

use anyhow::anyhow;
use axum::{
    extract::Path,
    response::{IntoResponse, Response},
    Extension,
};
use axum_extra::extract::PrivateCookieJar;
use chrono::Duration;
use hyper::StatusCode;
use mas_axum_utils::SessionInfoExt;
use mas_config::Encrypter;
use mas_data_model::{AuthorizationGrant, BrowserSession, TokenType};
use mas_router::{PostAuthAction, Route};
use mas_storage::{
    oauth2::{
        access_token::add_access_token,
        authorization_grant::{derive_session, fulfill_grant, get_grant_by_id},
        consent::fetch_client_consent,
        refresh_token::add_refresh_token,
    },
    user::ActiveSessionLookupError,
    PostgresqlBackend,
};
use mas_templates::Templates;
use oauth2_types::requests::{AccessTokenResponse, AuthorizationResponse};
use rand::thread_rng;
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;

use super::callback::{CallbackDestination, CallbackDestinationError, InvalidRedirectUriError};

#[derive(Debug, Error)]
pub enum RouteError {
    #[error(transparent)]
    Internal(Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error(transparent)]
    Anyhow(anyhow::Error),

    #[error("authorization grant is not in a pending state")]
    NotPending,
}

impl IntoResponse for RouteError {
    fn into_response(self) -> axum::response::Response {
        // TODO: better error pages
        match self {
            RouteError::NotPending => (
                StatusCode::BAD_REQUEST,
                "authorization grant not in a pending state",
            )
                .into_response(),
            RouteError::Anyhow(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
            RouteError::Internal(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
}

impl From<anyhow::Error> for RouteError {
    fn from(e: anyhow::Error) -> Self {
        Self::Anyhow(e)
    }
}

impl From<sqlx::Error> for RouteError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(Box::new(e))
    }
}

impl From<ActiveSessionLookupError> for RouteError {
    fn from(e: ActiveSessionLookupError) -> Self {
        Self::Internal(Box::new(e))
    }
}

impl From<InvalidRedirectUriError> for RouteError {
    fn from(e: InvalidRedirectUriError) -> Self {
        Self::Internal(Box::new(e))
    }
}

impl From<CallbackDestinationError> for RouteError {
    fn from(e: CallbackDestinationError) -> Self {
        Self::Internal(Box::new(e))
    }
}

pub(crate) async fn get(
    Extension(templates): Extension<Templates>,
    Extension(pool): Extension<PgPool>,
    cookie_jar: PrivateCookieJar<Encrypter>,
    Path(grant_id): Path<i64>,
) -> Result<Response, RouteError> {
    let mut txn = pool.begin().await?;

    let (session_info, cookie_jar) = cookie_jar.session_info();

    let maybe_session = session_info.load_session(&mut txn).await?;

    let grant = get_grant_by_id(&mut txn, grant_id).await?;

    let callback_destination = CallbackDestination::try_from(&grant)?;
    let continue_grant = PostAuthAction::continue_grant(grant_id);

    let session = if let Some(session) = maybe_session {
        session
    } else {
        // If there is no session, redirect to the login screen, redirecting here after
        // logout
        return Ok((cookie_jar, mas_router::Login::and_then(continue_grant).go()).into_response());
    };

    match complete(grant, session, txn).await {
        Ok(params) => {
            let res = callback_destination.go(&templates, params).await?;
            Ok((cookie_jar, res).into_response())
        }
        Err(GrantCompletionError::RequiresReauth) => Ok((
            cookie_jar,
            mas_router::Reauth::and_then(continue_grant).go(),
        )
            .into_response()),
        Err(GrantCompletionError::RequiresConsent) => {
            let next = mas_router::Consent(grant_id);
            Ok((cookie_jar, next.go()).into_response())
        }
        Err(GrantCompletionError::NotPending) => Err(RouteError::NotPending),
        Err(GrantCompletionError::Internal(e)) => Err(RouteError::Internal(e)),
        Err(GrantCompletionError::Anyhow(e)) => Err(RouteError::Anyhow(e)),
    }
}

#[derive(Debug, Error)]
pub enum GrantCompletionError {
    #[error(transparent)]
    Internal(Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error("authorization grant is not in a pending state")]
    NotPending,

    #[error("user needs to reauthenticate")]
    RequiresReauth,

    #[error("client lacks consent")]
    RequiresConsent,
}

impl From<sqlx::Error> for GrantCompletionError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(Box::new(e))
    }
}

impl From<InvalidRedirectUriError> for GrantCompletionError {
    fn from(e: InvalidRedirectUriError) -> Self {
        Self::Internal(Box::new(e))
    }
}

pub(crate) async fn complete(
    grant: AuthorizationGrant<PostgresqlBackend>,
    browser_session: BrowserSession<PostgresqlBackend>,
    mut txn: Transaction<'_, Postgres>,
) -> Result<AuthorizationResponse<Option<AccessTokenResponse>>, GrantCompletionError> {
    // Verify that the grant is in a pending stage
    if !grant.stage.is_pending() {
        return Err(GrantCompletionError::NotPending);
    }

    // Check if the authentication is fresh enough
    if !browser_session.was_authenticated_after(grant.max_auth_time()) {
        txn.commit().await?;
        return Err(GrantCompletionError::RequiresReauth);
    }

    let current_consent =
        fetch_client_consent(&mut txn, &browser_session.user, &grant.client).await?;

    let lacks_consent = grant
        .scope
        .difference(&current_consent)
        .any(|scope| !scope.starts_with("urn:matrix:device:"));

    // Check if the client lacks consent *or* if consent was explicitely asked
    if lacks_consent || grant.requires_consent {
        txn.commit().await?;
        return Err(GrantCompletionError::RequiresConsent);
    }

    // All good, let's start the session
    let session = derive_session(&mut txn, &grant, browser_session).await?;

    let grant = fulfill_grant(&mut txn, grant, session.clone()).await?;

    // Yep! Let's complete the auth now
    let mut params = AuthorizationResponse::default();

    // Did they request an auth code?
    if let Some(code) = grant.code {
        params.code = Some(code.code);
    }

    // Did they request an access token?
    // TODO: maybe we don't want to support the implicit flows
    if grant.response_type_token {
        let ttl = Duration::minutes(5);
        let (access_token_str, refresh_token_str) = {
            let mut rng = thread_rng();
            (
                TokenType::AccessToken.generate(&mut rng),
                TokenType::RefreshToken.generate(&mut rng),
            )
        };

        let access_token = add_access_token(&mut txn, &session, &access_token_str, ttl).await?;

        let _refresh_token =
            add_refresh_token(&mut txn, &session, access_token, &refresh_token_str).await?;

        params.response = Some(
            AccessTokenResponse::new(access_token_str)
                .with_expires_in(ttl)
                .with_refresh_token(refresh_token_str),
        );
    }

    // Did they request an ID token?
    if grant.response_type_id_token {
        return Err(anyhow!("id tokens are not implemented yet").into());
    }

    txn.commit().await?;
    Ok(params)
}

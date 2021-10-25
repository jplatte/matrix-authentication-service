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

#![allow(clippy::unused_async)]

use std::{
    convert::{TryFrom, TryInto},
    num::NonZeroU32,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use mas_data_model::{
    Authentication, AuthorizationCode, AuthorizationGrant, AuthorizationGrantStage, BrowserSession,
    Client, Pkce, Session, User,
};
use oauth2_types::{pkce::CodeChallengeMethod, requests::ResponseMode, scope::Scope};
use sqlx::PgExecutor;
use url::Url;

use crate::storage::{DatabaseInconsistencyError, IdAndCreationTime, PostgresqlBackend};

#[allow(clippy::too_many_arguments)]
pub async fn new_authorization_grant(
    executor: impl PgExecutor<'_>,
    client_id: String,
    redirect_uri: Url,
    scope: Scope,
    code: Option<AuthorizationCode>,
    state: Option<String>,
    nonce: Option<String>,
    max_age: Option<NonZeroU32>,
    acr_values: Option<String>,
    response_mode: ResponseMode,
    response_type_token: bool,
    response_type_id_token: bool,
) -> anyhow::Result<AuthorizationGrant<PostgresqlBackend>> {
    let code_challenge = code
        .as_ref()
        .and_then(|c| c.pkce.as_ref())
        .map(|p| &p.challenge);
    let code_challenge_method = code
        .as_ref()
        .and_then(|c| c.pkce.as_ref())
        .map(|p| p.challenge_method.to_string());
    let code_str = code.as_ref().map(|c| &c.code);
    let res = sqlx::query_as!(
        IdAndCreationTime,
        r#"
            INSERT INTO oauth2_authorization_grants
                (client_id, redirect_uri, scope, state, nonce, max_age,
                 acr_values, response_mode, code_challenge, code_challenge_method,
                 response_type_code, response_type_token, response_type_id_token,
                 code)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            RETURNING id, created_at
        "#,
        &client_id,
        redirect_uri.to_string(),
        scope.to_string(),
        state,
        nonce,
        // TODO: this conversion is a bit ugly
        max_age.map(|x| i32::try_from(u32::from(x)).unwrap_or(i32::MAX)),
        acr_values,
        response_mode.to_string(),
        code_challenge,
        code_challenge_method,
        code.is_some(),
        response_type_token,
        response_type_id_token,
        code_str,
    )
    .fetch_one(executor)
    .await
    .context("could not insert oauth2 authorization grant")?;

    let client = Client {
        data: (),
        client_id,
    };

    Ok(AuthorizationGrant {
        data: res.id,
        stage: AuthorizationGrantStage::Pending,
        code,
        redirect_uri,
        client,
        scope,
        state,
        nonce,
        max_age,
        acr_values,
        response_mode,
        created_at: res.created_at,
        response_type_token,
        response_type_id_token,
    })
}

struct GrantLookup {
    grant_id: i64,
    grant_created_at: DateTime<Utc>,
    grant_cancelled_at: Option<DateTime<Utc>>,
    grant_fulfilled_at: Option<DateTime<Utc>>,
    grant_exchanged_at: Option<DateTime<Utc>>,
    grant_scope: String,
    grant_state: Option<String>,
    grant_redirect_uri: String,
    grant_response_mode: String,
    grant_nonce: Option<String>,
    #[allow(dead_code)]
    grant_max_age: Option<i32>,
    grant_acr_values: Option<String>,
    grant_response_type_code: bool,
    grant_response_type_token: bool,
    grant_response_type_id_token: bool,
    grant_code: Option<String>,
    grant_code_challenge: Option<String>,
    grant_code_challenge_method: Option<String>,
    client_id: String,
    session_id: Option<i64>,
    user_session_id: Option<i64>,
    user_session_created_at: Option<DateTime<Utc>>,
    user_id: Option<i64>,
    user_username: Option<String>,
    user_session_last_authentication_id: Option<i64>,
    user_session_last_authentication_created_at: Option<DateTime<Utc>>,
}

impl TryInto<AuthorizationGrant<PostgresqlBackend>> for GrantLookup {
    type Error = DatabaseInconsistencyError;

    #[allow(clippy::too_many_lines)]
    fn try_into(self) -> Result<AuthorizationGrant<PostgresqlBackend>, Self::Error> {
        let scope: Scope = self
            .grant_scope
            .parse()
            .map_err(|_e| DatabaseInconsistencyError)?;

        let client = Client {
            data: (),
            client_id: self.client_id,
        };

        let last_authentication = match (
            self.user_session_last_authentication_id,
            self.user_session_last_authentication_created_at,
        ) {
            (Some(id), Some(created_at)) => Some(Authentication {
                data: id,
                created_at,
            }),
            (None, None) => None,
            _ => return Err(DatabaseInconsistencyError),
        };

        let session = match (
            self.session_id,
            self.user_session_id,
            self.user_session_created_at,
            self.user_id,
            self.user_username,
            last_authentication,
        ) {
            (
                Some(session_id),
                Some(user_session_id),
                Some(user_session_created_at),
                Some(user_id),
                Some(user_username),
                last_authentication,
            ) => {
                let user = User {
                    data: user_id,
                    username: user_username,
                    sub: format!("fake-sub-{}", user_id),
                };

                let browser_session = BrowserSession {
                    data: user_session_id,
                    user,
                    created_at: user_session_created_at,
                    last_authentication,
                };

                let client = client.clone();
                let scope = scope.clone();

                let session = Session {
                    data: session_id,
                    client,
                    browser_session,
                    scope,
                };

                Some(session)
            }
            (None, None, None, None, None, None) => None,
            _ => return Err(DatabaseInconsistencyError),
        };

        let stage = match (
            self.grant_fulfilled_at,
            self.grant_exchanged_at,
            self.grant_cancelled_at,
            session,
        ) {
            (None, None, None, None) => AuthorizationGrantStage::Pending,
            (Some(fulfilled_at), None, None, Some(session)) => AuthorizationGrantStage::Fulfilled {
                session,
                fulfilled_at,
            },
            (Some(fulfilled_at), Some(exchanged_at), None, Some(session)) => {
                AuthorizationGrantStage::Exchanged {
                    session,
                    fulfilled_at,
                    exchanged_at,
                }
            }
            (None, None, Some(cancelled_at), None) => {
                AuthorizationGrantStage::Cancelled { cancelled_at }
            }
            _ => {
                return Err(DatabaseInconsistencyError);
            }
        };

        let pkce = match (self.grant_code_challenge, self.grant_code_challenge_method) {
            (Some(challenge), Some(challenge_method)) if challenge_method == "plain" => {
                Some(Pkce {
                    challenge_method: CodeChallengeMethod::Plain,
                    challenge,
                })
            }
            (Some(challenge), Some(challenge_method)) if challenge_method == "S256" => Some(Pkce {
                challenge_method: CodeChallengeMethod::S256,
                challenge,
            }),
            (None, None) => None,
            _ => {
                return Err(DatabaseInconsistencyError);
            }
        };

        let code: Option<AuthorizationCode> =
            match (self.grant_response_type_code, self.grant_code, pkce) {
                (false, None, None) => None,
                (true, Some(code), pkce) => Some(AuthorizationCode { code, pkce }),
                _ => {
                    return Err(DatabaseInconsistencyError);
                }
            };

        let redirect_uri = self
            .grant_redirect_uri
            .parse()
            .map_err(|_e| DatabaseInconsistencyError)?;

        let response_mode = self
            .grant_response_mode
            .parse()
            .map_err(|_e| DatabaseInconsistencyError)?;

        Ok(AuthorizationGrant {
            data: self.grant_id,
            stage,
            client,
            code,
            acr_values: self.grant_acr_values,
            scope,
            state: self.grant_state,
            nonce: self.grant_nonce,
            max_age: None, // TODO
            response_mode,
            redirect_uri,
            created_at: self.grant_created_at,
            response_type_token: self.grant_response_type_token,
            response_type_id_token: self.grant_response_type_id_token,
        })
    }
}

pub async fn get_grant_by_id(
    executor: impl PgExecutor<'_>,
    id: i64,
) -> anyhow::Result<AuthorizationGrant<PostgresqlBackend>> {
    let res = sqlx::query_as!(
        GrantLookup,
        r#"
            SELECT
                og.id            AS grant_id,
                og.created_at    AS grant_created_at,
                og.cancelled_at  AS grant_cancelled_at,
                og.fulfilled_at  AS grant_fulfilled_at,
                og.exchanged_at  AS grant_exchanged_at,
                og.scope         AS grant_scope,
                og.state         AS grant_state,
                og.redirect_uri  AS grant_redirect_uri,
                og.response_mode AS grant_response_mode,
                og.nonce         AS grant_nonce,
                og.max_age       AS grant_max_age,
                og.acr_values    AS grant_acr_values,
                og.client_id     AS client_id,
                og.code          AS grant_code,
                og.response_type_code     AS grant_response_type_code,
                og.response_type_token    AS grant_response_type_token,
                og.response_type_id_token AS grant_response_type_id_token,
                og.code_challenge         AS grant_code_challenge,
                og.code_challenge_method  AS grant_code_challenge_method,
                os.id              AS "session_id?",
                us.id              AS "user_session_id?",
                us.created_at      AS "user_session_created_at?",
                 u.id              AS "user_id?",
                 u.username        AS "user_username?",
                usa.id             AS "user_session_last_authentication_id?",
                usa.created_at     AS "user_session_last_authentication_created_at?"
            FROM
                oauth2_authorization_grants og
            LEFT JOIN oauth2_sessions os
                ON os.id = og.oauth2_session_id
            LEFT JOIN user_sessions us
              ON us.id = os.user_session_id
            LEFT JOIN users u
              ON u.id = us.user_id
            LEFT JOIN user_session_authentications usa
              ON usa.session_id = us.id
            WHERE
                og.id = $1
        "#,
        id,
    )
    .fetch_one(executor)
    .await?;

    let grant = res.try_into()?;

    Ok(grant)
}

pub async fn lookup_grant_by_code(
    executor: impl PgExecutor<'_>,
    code: &str,
) -> anyhow::Result<AuthorizationGrant<PostgresqlBackend>> {
    let res = sqlx::query_as!(
        GrantLookup,
        r#"
            SELECT
                og.id            AS grant_id,
                og.created_at    AS grant_created_at,
                og.cancelled_at  AS grant_cancelled_at,
                og.fulfilled_at  AS grant_fulfilled_at,
                og.exchanged_at  AS grant_exchanged_at,
                og.scope         AS grant_scope,
                og.state         AS grant_state,
                og.redirect_uri  AS grant_redirect_uri,
                og.response_mode AS grant_response_mode,
                og.nonce         AS grant_nonce,
                og.max_age       AS grant_max_age,
                og.acr_values    AS grant_acr_values,
                og.client_id     AS client_id,
                og.code          AS grant_code,
                og.response_type_code     AS grant_response_type_code,
                og.response_type_token    AS grant_response_type_token,
                og.response_type_id_token AS grant_response_type_id_token,
                og.code_challenge         AS grant_code_challenge,
                og.code_challenge_method  AS grant_code_challenge_method,
                os.id              AS "session_id?",
                us.id              AS "user_session_id?",
                us.created_at      AS "user_session_created_at?",
                 u.id              AS "user_id?",
                 u.username        AS "user_username?",
                usa.id             AS "user_session_last_authentication_id?",
                usa.created_at     AS "user_session_last_authentication_created_at?"
            FROM
                oauth2_authorization_grants og
            LEFT JOIN oauth2_sessions os
                ON os.id = og.oauth2_session_id
            LEFT JOIN user_sessions us
              ON us.id = os.user_session_id
            LEFT JOIN users u
              ON u.id = us.user_id
            LEFT JOIN user_session_authentications usa
              ON usa.session_id = us.id
            WHERE
                og.code = $1
        "#,
        code,
    )
    .fetch_one(executor)
    .await?;

    let grant = res.try_into()?;

    Ok(grant)
}

pub async fn fulfill_grant(
    _executor: impl PgExecutor<'_>,
    _grant: AuthorizationGrant<PostgresqlBackend>,
    _session: BrowserSession<PostgresqlBackend>,
) -> anyhow::Result<AuthorizationGrant<PostgresqlBackend>> {
    // TODO: generate the session and attach it to the grant
    todo!()
}

pub async fn exchange_grant(
    _executor: impl PgExecutor<'_>,
    _grant: AuthorizationGrant<PostgresqlBackend>,
) -> anyhow::Result<AuthorizationGrant<PostgresqlBackend>> {
    // TODO: mark the grant as exchanged
    todo!()
}

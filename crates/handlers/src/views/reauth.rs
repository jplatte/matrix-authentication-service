// Copyright 2021, 2022 The Matrix.org Foundation C.I.C.
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

use axum::{
    extract::{Extension, Form, Query},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::PrivateCookieJar;
use mas_axum_utils::{
    csrf::{CsrfExt, ProtectedForm},
    fancy_error, FancyError, SessionInfoExt,
};
use mas_config::Encrypter;
use mas_router::Route;
use mas_storage::user::authenticate_session;
use mas_templates::{ReauthContext, TemplateContext, Templates};
use serde::Deserialize;
use sqlx::PgPool;

use super::shared::OptionalPostAuthAction;

#[derive(Deserialize, Debug)]
pub(crate) struct ReauthForm {
    password: String,
}

pub(crate) async fn get(
    Extension(templates): Extension<Templates>,
    Extension(pool): Extension<PgPool>,
    Query(query): Query<OptionalPostAuthAction>,
    cookie_jar: PrivateCookieJar<Encrypter>,
) -> Result<Response, FancyError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(fancy_error(templates.clone()))?;

    let (csrf_token, cookie_jar) = cookie_jar.csrf_token();
    let (session_info, cookie_jar) = cookie_jar.session_info();

    let maybe_session = session_info
        .load_session(&mut conn)
        .await
        .map_err(fancy_error(templates.clone()))?;

    let session = if let Some(session) = maybe_session {
        session
    } else {
        // If there is no session, redirect to the login screen, keeping the
        // PostAuthAction
        let login = mas_router::Login::from(query.post_auth_action);
        return Ok((cookie_jar, login.go()).into_response());
    };

    let ctx = ReauthContext::default();
    let next = query
        .load_context(&mut conn)
        .await
        .map_err(fancy_error(templates.clone()))?;
    let ctx = if let Some(next) = next {
        ctx.with_post_action(next)
    } else {
        ctx
    };
    let ctx = ctx.with_session(session).with_csrf(csrf_token.form_value());

    let content = templates
        .render_reauth(&ctx)
        .await
        .map_err(fancy_error(templates.clone()))?;

    Ok((cookie_jar, Html(content)).into_response())
}

pub(crate) async fn post(
    Extension(templates): Extension<Templates>,
    Extension(pool): Extension<PgPool>,
    Query(query): Query<OptionalPostAuthAction>,
    cookie_jar: PrivateCookieJar<Encrypter>,
    Form(form): Form<ProtectedForm<ReauthForm>>,
) -> Result<Response, FancyError> {
    let mut txn = pool.begin().await.map_err(fancy_error(templates.clone()))?;

    let form = cookie_jar
        .verify_form(form)
        .map_err(fancy_error(templates.clone()))?;

    let (session_info, cookie_jar) = cookie_jar.session_info();

    let maybe_session = session_info
        .load_session(&mut txn)
        .await
        .map_err(fancy_error(templates.clone()))?;

    let mut session = if let Some(session) = maybe_session {
        session
    } else {
        // If there is no session, redirect to the login screen, keeping the
        // PostAuthAction
        let login = mas_router::Login::from(query.post_auth_action);
        return Ok((cookie_jar, login.go()).into_response());
    };

    // TODO: recover from errors here
    authenticate_session(&mut txn, &mut session, form.password)
        .await
        .map_err(fancy_error(templates.clone()))?;
    let cookie_jar = cookie_jar.set_session(&session);
    txn.commit().await.map_err(fancy_error(templates.clone()))?;

    let reply = query.go_next();
    Ok((cookie_jar, reply).into_response())
}

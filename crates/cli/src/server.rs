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

use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use anyhow::Context;
use clap::Clap;
use hyper::{header, Server};
use mas_config::RootConfig;
use mas_core::{
    storage::MIGRATOR,
    tasks::{self, TaskQueue},
    templates::Templates,
};
use tower::{make::Shared, ServiceBuilder};
use tower_http::{
    compression::CompressionLayer,
    sensitive_headers::SetSensitiveHeadersLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::info;

use super::RootCommand;

#[derive(Clap, Debug, Default)]
pub(super) struct ServerCommand {
    /// Automatically apply pending migrations
    #[clap(long)]
    migrate: bool,
}

impl ServerCommand {
    pub async fn run(&self, root: &RootCommand) -> anyhow::Result<()> {
        let config: RootConfig = root.load_config()?;

        let addr: SocketAddr = config.http.address.parse()?;
        let listener = TcpListener::bind(addr)?;

        // Connect to the database
        let pool = config.database.connect().await?;

        if self.migrate {
            info!("Running pending migrations");
            MIGRATOR
                .run(&pool)
                .await
                .context("could not run migrations")?;
        }

        info!("Starting task scheduler");
        let queue = TaskQueue::default();
        queue.recuring(Duration::from_secs(15), tasks::cleanup_expired(&pool));
        queue.start();

        // Load and compile the templates
        // TODO: custom template path from the config
        let templates = Templates::load(None, true).context("could not load templates")?;

        // Start the server
        let root = mas_core::handlers::root(&pool, &templates, &config);

        let warp_service = warp::service(root);

        let service = ServiceBuilder::new()
            // Add high level tracing/logging to all requests
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().include_headers(true))
                    .on_response(
                        DefaultOnResponse::new()
                            .include_headers(true)
                            .latency_unit(LatencyUnit::Micros),
                    ),
            )
            // Set a timeout
            .timeout(Duration::from_secs(10))
            // Compress responses
            .layer(CompressionLayer::new())
            // Mark the `Authorization` and `Cookie` headers as sensitive so it doesn't show in logs
            .layer(SetSensitiveHeadersLayer::new(vec![
                header::AUTHORIZATION,
                header::COOKIE,
            ]))
            .service(warp_service);

        info!("Listening on http://{}", listener.local_addr().unwrap());

        Server::from_tcp(listener)?
            .serve(Shared::new(service))
            .await?;

        Ok(())
    }
}
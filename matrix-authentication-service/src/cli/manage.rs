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

use argon2::Argon2;
use clap::Clap;
use tracing::info;

use super::RootCommand;
use crate::{config::DatabaseConfig, storage::register_user};

#[derive(Clap, Debug)]
pub(super) struct ManageCommand {
    #[clap(subcommand)]
    subcommand: ManageSubcommand,
}

#[derive(Clap, Debug)]
enum ManageSubcommand {
    /// Register a new user
    Register { username: String, password: String },
}

impl ManageCommand {
    pub async fn run(&self, root: &RootCommand) -> anyhow::Result<()> {
        use ManageSubcommand as SC;
        match &self.subcommand {
            SC::Register { username, password } => {
                let config: DatabaseConfig = root.load_config()?;
                let pool = config.connect().await?;
                let hasher = Argon2::default();

                let user = register_user(&pool, hasher, username, password).await?;
                info!(?user, "User registered");

                Ok(())
            }
        }
    }
}
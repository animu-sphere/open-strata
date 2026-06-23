// SPDX-License-Identifier: Apache-2.0
//! Loading extension manifests (embedded built-ins + user overlay).

use std::collections::BTreeMap;

use ost_core::paths::Store;
use ost_core::{Error, Result};

use crate::model::Extension;

const BUILTINS: &[(&str, &str)] = &[
    ("openusd", include_str!("../../../extensions/openusd.yaml")),
    ("materialx", include_str!("../../../extensions/materialx.yaml")),
];

/// All known extensions, keyed and ordered by id.
pub struct Catalog {
    extensions: BTreeMap<String, Extension>,
}

impl Catalog {
    pub fn load() -> Result<Catalog> {
        let user_dir = Store::discover().extensions();
        let extensions = ost_core::catalog::load(BUILTINS, &user_dir, parse)?;
        Ok(Catalog { extensions })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Extension> {
        self.extensions.values()
    }

    pub fn get(&self, id: &str) -> Option<&Extension> {
        self.extensions.get(id)
    }
}

fn parse(label: &str, src: &str) -> Result<Extension> {
    serde_yaml::from_str(src).map_err(|err| Error::parse(format!("extension '{label}'"), err))
}

pub fn load_all() -> Result<Catalog> {
    Catalog::load()
}

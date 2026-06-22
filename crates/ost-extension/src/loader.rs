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
        let mut extensions = BTreeMap::new();
        for (id, src) in BUILTINS {
            let e = parse(id, src)?;
            extensions.insert(e.id.clone(), e);
        }

        let user_dir = Store::discover().extensions();
        if user_dir.as_std_path().is_dir() {
            let entries = std::fs::read_dir(user_dir.as_std_path())
                .map_err(|e| Error::io(user_dir.to_string(), e))?;
            for entry in entries {
                let entry = entry.map_err(|e| Error::io(user_dir.to_string(), e))?;
                let path = entry.path();
                let is_yaml = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e == "yaml" || e == "yml")
                    .unwrap_or(false);
                if !is_yaml {
                    continue;
                }
                let src = std::fs::read_to_string(&path)
                    .map_err(|e| Error::io(path.display().to_string(), e))?;
                let e = parse(&path.display().to_string(), &src)?;
                extensions.insert(e.id.clone(), e);
            }
        }

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
    let e: Extension =
        serde_yaml::from_str(src).map_err(|err| Error::parse(format!("extension '{label}'"), err))?;
    if e.id.is_empty() {
        return Err(Error::InvalidManifest(format!(
            "extension '{label}' is missing an 'id'"
        )));
    }
    Ok(e)
}

pub fn load_all() -> Result<Catalog> {
    Catalog::load()
}

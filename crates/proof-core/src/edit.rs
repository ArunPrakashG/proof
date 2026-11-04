//! Changes to the map that keep its formatting and comments.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

use crate::slug;

fn open(path: &Path) -> Result<DocumentMut> {
    Ok(std::fs::read_to_string(path)?.parse::<DocumentMut>()?)
}

fn caps(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    doc.get_mut("capability").and_then(Item::as_array_of_tables_mut).ok_or_else(|| anyhow!("the map has no capabilities"))
}

fn find<'a>(caps: &'a mut ArrayOfTables, id: &str) -> Result<&'a mut Table> {
    caps.iter_mut()
        .find(|t| {
            let g = t.get("group").and_then(Item::as_str).unwrap_or_default();
            let n = t.get("name").and_then(Item::as_str).unwrap_or_default();
            slug(&[g, n]) == id
        })
        .ok_or_else(|| anyhow!("no capability {id}"))
}

/// A human has read this capability: drop its `inferred` flag.
pub fn review(path: &Path, id: &str) -> Result<()> {
    let mut doc = open(path)?;
    find(caps(&mut doc)?, id)?.remove("inferred");
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

/// Add files to a capability's `code`.
pub fn claim(path: &Path, id: &str, files: &[String]) -> Result<usize> {
    let mut doc = open(path)?;
    let cap = find(caps(&mut doc)?, id)?;
    if cap.get("code").is_none() {
        cap.insert("code", value(Array::new()));
    }
    let code = cap.get_mut("code").and_then(Item::as_array_mut).ok_or_else(|| anyhow!("{id}: code isn't a list"))?;
    let mut added = 0;
    for f in files {
        if !code.iter().any(|v| v.as_str() == Some(f)) {
            code.push(f.as_str());
            added += 1;
        }
    }
    std::fs::write(path, doc.to_string())?;
    Ok(added)
}

/// Add a capability that claims `files`. Returns its id.
pub fn new_capability(path: &Path, group: &str, name: &str, files: &[String], inferred: bool) -> Result<String> {
    let id = slug(&[group, name]);
    if group.trim().is_empty() || name.trim().is_empty() {
        bail!("a capability needs a group and a name");
    }
    let mut doc = open(path)?;
    if doc.get("capability").is_none() {
        doc.insert("capability", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let list = caps(&mut doc)?;
    if find(list, &id).is_ok() {
        bail!("{group} › {name} is already in the map");
    }
    let mut t = Table::new();
    t.insert("group", value(group.trim()));
    t.insert("name", value(name.trim()));
    if inferred {
        t.insert("inferred", value(true));
    }
    let mut code = Array::new();
    for f in files {
        code.push(f.as_str());
    }
    t.insert("code", value(code));
    list.push(t);
    std::fs::write(path, doc.to_string())?;
    Ok(id)
}

/// `Story runs › Extra life` or `Story runs > Extra life` -> (group, name).
pub fn split_title(title: &str) -> Option<(String, String)> {
    let (g, n) = title.split_once('›').or_else(|| title.split_once('>'))?;
    Some((g.trim().to_string(), n.trim().to_string())).filter(|(g, n)| !g.is_empty() && !n.is_empty())
}

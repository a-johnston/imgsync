use std::collections::HashMap;
use std::fs::create_dir_all;
use std::hash::Hash;
use std::path::{Path, PathBuf};

pub fn map_update<'a, K, V>(
    map_a: &'a mut HashMap<K, V>,
    map_b: &'a mut HashMap<K, V>,
    update: impl Fn(&mut V, &V),
) -> &'a HashMap<K, V>
where
    K: Eq + Hash,
{
    map_b.drain().for_each(|(k, v)| {
        map_a.entry(k).and_modify(|g| update(g, &v)).or_insert(v);
    });
    map_a
}

pub fn format_bytes(mut val: f64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut unit = 0;
    while unit < UNITS.len() - 1 && val > 1000.0 {
        unit += 1;
        val /= 1000.0;
    }
    format!("{val:.1} {}", UNITS[unit])
}

pub fn expand_homedir(path: &str) -> String {
    path.replace("~", std::env::var("HOME").unwrap().as_str())
}

pub fn path_fname<P>(p: &P) -> &str
where
    P: AsRef<Path> + ?Sized,
{
    p.as_ref().file_name().unwrap().to_str().unwrap()
}

pub fn path_with_push<T: AsRef<Path>>(path: &PathBuf, push: T) -> PathBuf {
    let mut new = path.clone();
    new.push(push);
    new
}

pub fn create_parents(path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)
    } else {
        Ok(())
    }
}

pub fn get_user_confirmation(msg: &str, def: bool) -> bool {
    println!(
        "{msg}: ('y' or 'yes' to accept, empty defaults to {})",
        if def { "accept" } else { "reject" }
    );
    let mut buf: String = String::new();
    let stdin = std::io::stdin();
    stdin.read_line(&mut buf).expect("Readline failed");
    let lower = buf.trim_end_matches('\n').to_lowercase();
    (lower.is_empty() && def) || lower == "y" || lower == "yes"
}

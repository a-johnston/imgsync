use std::collections::{HashMap, HashSet};
use std::fs::{create_dir_all, exists, read_to_string, write};
use std::path::PathBuf;
use std::time::SystemTime;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Deserialize;

mod files;
use files::{Destination, FileGroup, FileInfo, FilePattern};
mod plan;
mod util;

const CONFIG_PATH: &str = "~/.config/imgsync/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../default_config.toml");

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct Config {
    sources: Vec<FilePattern>,
    destinations: Vec<Destination>,
    #[serde(default = "default_true")]
    log_moves: bool,
    #[serde(default = "default_true")]
    confirm_moves: bool,
    #[serde(default)]
    prefer_dest_copies: bool,
}

impl Config {
    fn load() -> Self {
        let path = PathBuf::from(util::expand_homedir(CONFIG_PATH));
        if !exists(&path).unwrap_or(false) {
            println!("Writing default config to {path:?}");
            util::create_parents(&path).expect("Failed to create config directory");
            write(&path, DEFAULT_CONFIG).expect("Failed to write default config");
        } else {
            println!("Using config at {path:?}");
        }
        let content = read_to_string(&path).expect("Failed to read config");
        let mut config: Self = toml::from_str(&content).expect("Failed to load config data");
        (config.sources)
            .iter_mut()
            .for_each(|source| source.expand_paths());
        (config.destinations)
            .iter_mut()
            .for_each(|dest| dest.expand_paths());
        config
    }
}

fn get_dest_dirs(config: &Config, groups: &Vec<FileGroup>) -> HashSet<PathBuf> {
    (config.destinations)
        .par_iter()
        .flat_map(|dest| dest.get_dirs(groups))
        .collect()
}

fn main() {
    let start = SystemTime::now();
    let config = Config::load();
    if config.sources.is_empty() && config.destinations.is_empty() {
        println!("Nothing to do (hint: no sources or destinations configured)");
        return;
    }
    let mut groups = files::get_groups(
        (config.sources)
            .par_iter()
            .flat_map(|source| source.match_files()),
    );
    let file_total: usize = groups.iter().map(|g| g.files.len()).sum();
    println!(
        "Found {file_total} source files and {} file groups in {:.1?}",
        groups.len(),
        start.elapsed().unwrap()
    );
    if file_total == 0 {
        println!("Nothing to do");
        return;
    }
    let dest_dirs = get_dest_dirs(&config, &groups);
    let mut existing: HashMap<_, _> = files::get_files_for_dirs(dest_dirs.par_iter())
        .filter_map(|dir_entry| FileInfo::get_from_dir_entry(dir_entry, &None))
        .map(|f| (f.path.clone(), f))
        .collect();
    let mut plan = plan::Plan::new(config.prefer_dest_copies);
    for dest in config.destinations {
        dest.plan_moves(&mut plan, &mut groups, &mut existing);
    }
    println!("Finished planning in {:.1?}", start.elapsed().unwrap());
    let total_moves = plan.len();
    if total_moves == 0 {
        println!("Nothing to do");
        return;
    }
    if config.log_moves {
        plan.log_moves();
    }
    println!("Total moves: {total_moves}");
    if config.confirm_moves && !util::get_user_confirmation("Confirm moves", false) {
        println!("Quitting");
        return;
    }
    (dest_dirs.iter())
        .for_each(|d| create_dir_all(d).unwrap_or_else(|_| panic!("Failed to create dir {d:?}")));
    plan.perform_moves();
}

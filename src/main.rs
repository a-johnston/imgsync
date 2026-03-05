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
struct Section {
    name: Option<String>,
    sources: Vec<FilePattern>,
    destinations: Vec<Destination>,
}

impl Section {
    fn expand_paths(&mut self) {
        self.sources.iter_mut().for_each(|s| s.expand_paths());
        self.destinations.iter_mut().for_each(|d| d.expand_paths());
    }

    fn get_groups(&self) -> Vec<FileGroup> {
        (self.sources.par_iter())
            .flat_map(|source| files::get_groups(source.match_files()))
            .collect()
    }

    fn get_dest_dirs(&self, groups: &[FileGroup]) -> HashSet<PathBuf> {
        (self.destinations.par_iter())
            .flat_map(|dest| dest.get_dirs(groups))
            .collect()
    }
}

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    sections: Vec<Section>,
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
        config.sections.iter_mut().for_each(Section::expand_paths);
        config
    }
}

fn main() {
    let start = SystemTime::now();
    let config = Config::load();
    if config.sections.is_empty() {
        println!("Nothing to do (hint: no sources or destinations) configured)");
        return;
    }

    let mut plan = plan::Plan::new(config.prefer_dest_copies);
    let mut all_dest_dirs: HashSet<PathBuf> = HashSet::new();

    for (i, section) in config.sections.iter().enumerate() {
        let groups = section.get_groups();
        let file_total: usize = groups.iter().map(|g| g.files.len()).sum();
        println!(
            "[{}] Found {file_total} source files in {} groups",
            section.name.clone().unwrap_or_else(|| (i + 1).to_string()),
            groups.len()
        );
        if file_total == 0 {
            continue;
        }
        let dest_dirs = section.get_dest_dirs(&groups);
        let mut existing: HashMap<_, _> = files::get_files_for_dirs(dest_dirs.par_iter())
            .filter_map(|dir_entry| FileInfo::get_from_dir_entry(dir_entry, &None))
            .map(|f| (f.path.clone(), f))
            .collect();
        for dest in &section.destinations {
            dest.plan_moves(&mut plan, &groups, &mut existing);
        }
        all_dest_dirs.extend(dest_dirs);
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
    (all_dest_dirs.iter())
        .for_each(|d| create_dir_all(d).unwrap_or_else(|_| panic!("Failed to create dir {d:?}")));
    plan.perform_moves();
}

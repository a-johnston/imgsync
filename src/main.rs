use std::collections::{HashMap, HashSet};
use std::fs::{copy, create_dir_all, exists, read_to_string, write};
use std::path::PathBuf;
use std::time::SystemTime;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use serde::Deserialize;

mod files;
use files::{FileGroup, FileInfo};
mod util;

const CONFIG_PATH: &str = "~/.config/imgsync/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../default_config.toml");

#[derive(Deserialize, Debug)]
struct Config {
    source_dirs: Vec<String>,
    source_filename: String,
    dest_dirs: Vec<String>,
    dest_filename: String,
    log_moves: bool,
    confirm_moves: bool,
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
        util::expand_homedir_vec(&mut config.source_dirs);
        util::expand_homedir_vec(&mut config.dest_dirs);
        config
    }

    fn get_all_dest_dirs(&self, groups: &Vec<FileGroup>) -> HashSet<PathBuf> {
        (self.dest_dirs.iter())
            .flat_map(|dest| groups.iter().map(|g| g.get_dest_dir(dest)))
            .collect::<HashSet<_>>()
    }
}

fn populate_moves_for_dest(
    groups: &mut Vec<FileGroup>,
    dest_format: &str,
    file_format: &str,
    moves: &mut HashMap<PathBuf, FileInfo>,
    existing: &mut HashMap<PathBuf, FileInfo>,
) {
    for group in groups {
        group.ensure_unique(&existing, dest_format, file_format);
        for (path, file) in group.get_moves(dest_format, file_format) {
            if !existing.contains_key(&path) {
                moves.insert(path.clone(), file.clone());
                existing.insert(path.clone(), file.clone());
            }
        }
    }
}

fn log_moves(moves: &HashMap<PathBuf, FileInfo>) {
    println!("Moves:");
    let max_src_len = &(moves.iter())
        .map(|(_, f)| f.path.as_os_str().len())
        .max()
        .unwrap();
    let mut sorted_moves: Vec<_> = (moves.iter())
        .map(|(path, file)| (file.path.display().to_string(), path.display().to_string()))
        .collect();
    sorted_moves.sort();
    for (src, dest) in sorted_moves {
        println!("- {1:0$} -> {2}", max_src_len, src, dest);
    }
}

fn copy_files(moves: &HashMap<PathBuf, FileInfo>) -> u64 {
    (moves.par_iter())
        .map(|(dest, file)| {
            copy(&file.path, dest).unwrap_or_else(|e| {
                println!("Error copying {:?} to {dest:?}: {e:?}", &file.path);
                0
            })
        })
        .sum()
}

fn main() {
    let start = SystemTime::now();
    let config = Config::load();
    if config.source_dirs.len() == 0 || config.dest_dirs.len() == 0 {
        println!("Nothing to do (hint: no source or dest directory configured)");
        return;
    }
    let file_matcher = Regex::new(&config.source_filename).unwrap();
    let mut groups = files::get_groups(files::get_files_for_patterns(
        &config.source_dirs,
        Some(&file_matcher),
    ));
    let file_total: usize = groups.iter().map(|g| g.files.len()).sum();
    println!(
        "Found {} file groups in {:?} and {file_total} files",
        groups.len(),
        start.elapsed().unwrap()
    );
    let dest_dirs = config.get_all_dest_dirs(&groups);
    let mut existing: HashMap<_, _> = files::get_files_for_dirs(dest_dirs.par_iter(), None)
        .map(|f| (f.path.clone(), f))
        .collect();
    let mut moves: HashMap<PathBuf, FileInfo> = HashMap::new();
    for dest_format in &config.dest_dirs {
        populate_moves_for_dest(
            &mut groups,
            &dest_format,
            &config.dest_filename,
            &mut moves,
            &mut existing,
        );
    }
    println!("Finished planning in {:?}", start.elapsed().unwrap());
    if moves.is_empty() {
        println!("Nothing to do");
        return;
    }
    if config.log_moves {
        log_moves(&moves);
    }
    println!("Total moves: {}", &moves.len());
    if config.confirm_moves && !util::get_user_confirmation("Confirm moves", false) {
        println!("Quitting");
        return;
    }
    let copy_start = SystemTime::now();
    (dest_dirs.iter())
        .for_each(|d| create_dir_all(d).expect(format!("Failed to create dir {d:?}").as_str()));
    let total_size = copy_files(&moves);
    let copy_time = copy_start.elapsed().unwrap();
    println!(
        "Moved: {}\tTotal Time: {:.1?}\t Copy Time: {:.1?}\tCopy Rate: {}/s",
        moves.len(),
        start.elapsed().unwrap(),
        copy_time,
        util::format_bytes(total_size as f64 / copy_time.as_secs_f64())
    );
}

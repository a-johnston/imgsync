use std::collections::{HashMap, HashSet};
use std::fs::{DirEntry, copy, create_dir_all, exists, read_dir, read_to_string, write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use serde::Deserialize;

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
            .flat_map(|dest| get_group_dest_dirs(groups, dest).into_iter())
            .collect::<HashSet<_>>()
    }
}
#[derive(Clone)]
struct FileFormatArgs {
    name: String,
    parent: String,
    extensions: String,
}

impl FileFormatArgs {
    fn new(p: &PathBuf, matcher: Option<&Regex>) -> Option<Self> {
        let file_name = util::path_fname(p);
        let mut parent = (p.parent())
            .and_then(|p| Some(util::path_fname(p)))
            .unwrap_or("");
        let (mut name, mut extensions) = (file_name.split_once('.'))
            .and_then(|(a, b)| Some((a, b.trim_start_matches("."))))
            .unwrap_or((file_name, ""));
        if let Some(regex) = matcher {
            if let Some(captures) = regex.captures(file_name) {
                (captures.name("name")).inspect(|s| name = s.as_str());
                (captures.name("parent")).inspect(|s| parent = s.as_str());
                (captures.name("extensions")).inspect(|s| extensions = s.as_str());
            } else {
                return None;
            }
        }
        Some(Self {
            name: name.to_string(),
            parent: parent.to_string(),
            extensions: extensions.trim_start_matches(".").to_string(),
        })
    }

    fn format(&self, s: &str, suffix: &str) -> String {
        s.replace("{name}", &self.name)
            .replace("{parent}", &self.parent)
            .replace("{suffix}", &suffix)
            .replace("{extensions}", &self.extensions)
    }
}

#[derive(Clone)]
struct FileInfo {
    path: PathBuf,
    created: SystemTime,
    format_args: FileFormatArgs,
}

impl FileInfo {
    fn get_from_dir_entry(dir_entry: DirEntry, file_matcher: Option<&Regex>) -> Option<Self> {
        if let Ok(metadata) = dir_entry.metadata()
            && metadata.is_file()
            && let Some(format_args) = FileFormatArgs::new(&dir_entry.path(), file_matcher)
        {
            Some(Self {
                path: dir_entry.path(),
                // NB: If created metadata is not available, this program shouldn't run
                created: metadata.created().unwrap(),
                format_args: format_args,
            })
        } else {
            None
        }
    }

    fn matches(&self, other: &Self) -> bool {
        // This is a bad heuristic although it works for me. Replace/combine with cheap file hash?
        let delta = match self.created.duration_since(other.created) {
            Ok(d) => d,
            Err(e) => e.duration(),
        };
        return delta.as_secs_f32() < 0.001;
    }
}

struct FileGroup {
    files: Vec<FileInfo>,
    suffix: u64,
}

impl FileGroup {
    fn empty() -> Self {
        Self {
            files: vec![],
            suffix: 0,
        }
    }

    fn get_groups(files: impl ParallelIterator<Item = FileInfo>) -> Vec<Self> {
        files
            .fold(
                || HashMap::new(),
                |mut groups, file| {
                    let key = file.path.file_prefix().unwrap().to_owned();
                    let group = groups.entry(key).or_insert_with(FileGroup::empty);
                    group.files.push(file);
                    groups
                },
            )
            .reduce(
                || HashMap::new(),
                |mut map_a, mut map_b| {
                    util::map_update(&mut map_a, &mut map_b, |a, b| {
                        (*a).files.extend_from_slice(&b.files)
                    });
                    map_a
                },
            )
            .drain()
            .map(|(_, v)| v)
            .collect()
    }

    fn get_ts(&self) -> DateTime<Local> {
        // Uses the earliest available file creation time. Intuitively, editor
        // metadata is created after image files are, but needs to be moved to
        // the same destination for migrations.
        self.files.iter().map(|f| f.created).min().unwrap().into()
    }

    fn get_dest_dir(&self, dest_format: &str) -> PathBuf {
        PathBuf::from(self.get_ts().format(&dest_format).to_string().as_str())
    }

    fn get_moves(
        &self,
        dest_format: &str,
        file_format: &str,
    ) -> impl Iterator<Item = (PathBuf, &FileInfo)> {
        let dir = self.get_dest_dir(dest_format);
        let format = self.get_ts().format(file_format).to_string();
        self.files.iter().map(move |file| {
            let suffix = if self.suffix == 0 {
                String::new()
            } else {
                format!("_{}", self.suffix)
            };
            let file_name = file.format_args.format(&format, &suffix);
            (util::path_with_push(&dir, file_name.as_str()), file)
        })
    }

    fn ensure_unique(
        &mut self,
        existing: &HashMap<PathBuf, FileInfo>,
        dest_format: &str,
        file_format: &str,
    ) {
        while self.get_moves(dest_format, file_format).any(|(k, v)| {
            (existing.get(&k))
                .and_then(|other| Some(!v.matches(other)))
                .unwrap_or(false)
        }) {
            self.suffix += 1;
        }
    }
}

fn get_files_for_dir<P>(
    path: P,
    file_matcher: Option<&Regex>,
) -> impl ParallelIterator<Item = FileInfo>
where
    P: AsRef<Path>,
{
    (read_dir(path).ok().into_par_iter())
        .flat_map_iter(|paths| paths.filter_map(|e| e.ok()))
        .filter_map(move |entry| FileInfo::get_from_dir_entry(entry, file_matcher))
}

fn get_files_for_dirs<P>(
    dirs: impl ParallelIterator<Item = P>,
    file_matcher: Option<&Regex>,
) -> impl ParallelIterator<Item = FileInfo>
where
    P: AsRef<Path>,
{
    dirs.flat_map(move |d| get_files_for_dir(d, file_matcher))
}

fn get_files_for_pattern(
    pattern: &str,
    file_matcher: &Regex,
) -> impl ParallelIterator<Item = FileInfo> {
    (glob::glob(pattern).into_par_iter())
        .flat_map_iter(|paths| paths.filter_map(|res| res.ok()))
        .flat_map(move |dir| get_files_for_dir(dir, Some(file_matcher)))
}

fn get_files_for_patterns(
    patterns: &Vec<String>,
    file_matcher: &Regex,
) -> impl ParallelIterator<Item = FileInfo> {
    (patterns.par_iter()).flat_map(move |pattern| get_files_for_pattern(pattern, file_matcher))
}

fn get_group_dest_dirs(groups: &Vec<FileGroup>, dest: &str) -> HashSet<PathBuf> {
    groups.iter().map(|g| g.get_dest_dir(dest)).collect()
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
    let mut groups =
        FileGroup::get_groups(get_files_for_patterns(&config.source_dirs, &file_matcher));
    let file_total: usize = groups.iter().map(|g| g.files.len()).sum();
    println!(
        "Found {} file groups in {:?} and {file_total} files",
        groups.len(),
        start.elapsed().unwrap()
    );
    let dest_dirs = config.get_all_dest_dirs(&groups);
    let mut existing: HashMap<_, _> = get_files_for_dirs(dest_dirs.par_iter(), None)
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

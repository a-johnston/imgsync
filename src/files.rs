use std::collections::HashMap;
use std::fs::{DirEntry, read_dir};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use regex::Regex;

use super::util;

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
pub struct FileInfo {
    pub path: PathBuf,
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

pub struct FileGroup {
    pub files: Vec<FileInfo>,
    suffix: u64,
}

impl FileGroup {
    pub fn empty() -> Self {
        Self {
            files: vec![],
            suffix: 0,
        }
    }

    fn get_ts(&self) -> DateTime<Local> {
        // Uses the earliest available file creation time. Intuitively, editor
        // metadata is created after image files are, but needs to be moved to
        // the same destination for migrations.
        self.files.iter().map(|f| f.created).min().unwrap().into()
    }

    pub fn get_dest_dir(&self, dest_format: &str) -> PathBuf {
        PathBuf::from(self.get_ts().format(&dest_format).to_string().as_str())
    }

    pub fn get_moves(
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

    pub fn ensure_unique(
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

pub fn get_groups(files: impl ParallelIterator<Item = FileInfo>) -> Vec<FileGroup> {
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

fn get_files_for_dir<P>(
    path: P,
    file_matcher: Option<&Regex>,
) -> impl ParallelIterator<Item = FileInfo>
where
    P: AsRef<Path>,
{
    read_dir(path)
        .into_par_iter()
        .flatten_iter()
        .flatten_iter()
        .filter_map(move |entry| FileInfo::get_from_dir_entry(entry, file_matcher))
}

pub fn get_files_for_dirs<P>(
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
    file_matcher: Option<&Regex>,
) -> impl ParallelIterator<Item = FileInfo> {
    get_files_for_dirs(
        glob::glob(pattern)
            .into_par_iter()
            .flat_map_iter(|paths| paths.filter_map(|res| res.ok())),
        file_matcher,
    )
}

pub fn get_files_for_patterns(
    patterns: &Vec<String>,
    file_matcher: Option<&Regex>,
) -> impl ParallelIterator<Item = FileInfo> {
    patterns
        .par_iter()
        .flat_map(move |pattern| get_files_for_pattern(pattern, file_matcher))
}

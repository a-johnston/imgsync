# imgsync

Basic utility for copying new files found in sources to structured destinations.
The specific use case this was designed for is quickly syncing new photos from an
SD card into a folder-structured library as well as backup paths. This program only
reads and copies files - no files are removed or altered.

On first run, the binary creates a [config template](https://github.com/a-johnston/imgsync/blob/main/default_config.toml)
at `~/.config/imgsync/config.toml`- at least one source and destination need to be
configured before use. For example, a user on MacOS might have the following config:

```toml
[[sources]]
dir_pattern = "/Volumes/*/DCIM/*/"
file_regex = "^\\w+([.]\\w+)+$"

[[destinations]]
dir_pattern = "~/Pictures/library/%Y/%b/"
file_pattern = "{parent}_{name}{suffix}.{extensions}"

[[destinations.ignore]]
dir_pattern = "~/Pictures/rejects/"
file_regex = "^\\w+_\\w+([.]\\w+)+$"
```

# Multi-Destination Syncing

If multiple destinations are configured, in some cases it can be helpful to set
`prefer_dest_copies = true`. This option uses the first destination for each file
as a host for copying files to remaining desetinations, under the assumption that
the user's primary storage has higher read speeds compared to an SD card.

Destinations are processed in-order so this option benefits from faster storage
being earlier in the config. It does not affect the final set of files or their
contents.

# Migrating existing files

Named capture groups can be used within `[[sources]]` to override what values are used
for formatting destination paths. Using the above config as an example, if one wanted
to add an additional folder, a source can be added for the current destination to copy
those files to the new structure:

```toml
[[sources]]
dir_pattern = "/Volumes/*/DCIM/*/"
file_regex = "^\\w+([.]\\w+)+$"

[[sources]]
dir_pattern = "~/Pictures/old_library/*/*/"
file_regex = "^(?<parent>\\w+)_(?<name>\\w+)([.]\\w+)+$"

[[destinations]]
dir_pattern = "~/Pictures/new_library/%Y/%b/%d"
file_pattern = "{parent}_{name}{suffix}.{extensions}"
```

In this example, the named capture groups `parent` and `name` are matched by the regex
and used in place of the default values for that file (which are the name of the parent
directory and full file prefix respectively). The original source can optionally be left
enabled although currently there is no prioritization for which source file to copy if
two are determined to be the same file and as a result the migration might be faster if
that source was omitted.

# File Matching

If two files are mapped to the same destination, or that destination already exists,
imgsync uses a very basic heuristic to determine if those files should be considered
the same: if the files were created at the same time and if the files have the same
extension. This works for my use case but may cause problems with other use cases.
In the future, file size and content hash could be considered as well.

If two files are determined to be matching by these criteria, the copy is skipped.
Otherwise, a unique file suffix value is incremented until the filename is unique.

# File Grouping

When scanning source files, any files which have the same prefix are put into a group
together. When the files are moved, they share a common unique suffix (as determined
by the file uniqueness criteria described above) and are moved to the same destination
folder as determined by the earliest file creation time in the group. This process
ensures any metadata files created later, such as during editing, are moved alongside
the original source files.

# Todo

- Optimize copy planning for maximum throughput
  - Currently, `prefer_dest_copies` can improve throughput but because the copies are
    planned as two discrete steps, and because the first dest has greater bandwidth
    than the source, copies from the first dest can start being written to secondary
    dests as soon as they finish. Instead, this only starts after the entire first pass
    is complete. As a result, bandwidth use across the full plan is suboptimal for both
    the source and first dest. A good solution might be to invert the map of `<dest_path,
    source_file>` to a map of `<source_file, vec<dest_path>>`.
  - The current behavior of `prefer_dest_copies` does enable the source to be disconnected
    after the first pass (the first dest is determined by file, so a secondary dest missing
    files compared to the primary will be the "first dest" for those files and those copies
    will happen in the first pass). It's unclear if this is actually useful behavior but it
    would not be immediately compatible with the above bullet point's solution.
- Optional recovery corrupted files in the case of failed/interrupted copies
- Optional automatic deleting of migrated files
- More robust file matching

# Why?

Because I wanted a simple command to run when I plug in my SD cards without needing to start
up darktable. Also it was getting a bit ridiculous browsing my existing library, which was
just a single folder. I also wanted an excuse to use rust again so I wouldn't forget everything
about it.

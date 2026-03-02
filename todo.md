- Process sources sequentially to ensure the earliest source is used for prefer
  dest copies option
  - Also potentially rethink file grouping -- file prefix is simplistic, groups
    all hidden files together, and at the moment can span multiple sources
  - Probably just avoid grouping hidden files entirely? So empty prefix means
    no group?

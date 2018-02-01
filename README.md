# sourcesecrets

A tool for finding patterns of text in Git history

## rationale

Repository maintainers sometimes do not fully consider the impact of the "secret" they are redacting or may not realize they have committed the removal of a secret from source code without fully redacting it or mitigating the issue. `sourcesecrets` allows you to provide a file extension or regular expression to find in every single commit ever made to a repository to find secrets left in source code history.

## usage

```
sourcesecrets -o secrets.csv repos/*/
```

## improvements to be made

1. This application uses the `grep` crate. This was done entirely because ripgrep, a fast text search application, parses/evaluates regex and I decided to use the same crate they use. This may not be the best-fit regex crate for this scenario.
2. The `git` utility is invoked for *every* commit to get contents and other details. Using some `libgit2` bindings or another library may provide benefits over the overhead of invoking a new process for every commit.

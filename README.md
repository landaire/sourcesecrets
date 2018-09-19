# sourcesecrets

A tool for finding patterns of text in Git history

## Rationale

Repository maintainers sometimes do not fully consider the impact of the "secret" they are redacting or may not realize they have committed the removal of a secret from source code without fully redacting it or mitigating the issue. `sourcesecrets` allows you to provide a file extension or regular expression to find in every single commit ever made to a repository to find secrets left in source code history.

## Usage

Example 1:

```
sourcesecrets -o secrets.csv repos/*/
```

Example 2:

```
sourcesecrets -o secrets.csv -d definitions.private.toml repo_path
```

## Defining patterns

Patterns you want to have hits on need be defined in a TOML file and either placed in the application's executable directory or provided with the `-d/--definitions` flag on the command line. An example definitions file looks like so:

```toml
[[patterns]]
description = "Password in code"
pattern = "Password = \"[^\"]+\"[^;]+"

[[files]]
description = "Private key file"
extension = "pfx"
binary = true

[[filters]]
description = "Remove bad hits in documentation"
pattern = "</param>"
```

The patterns section defines content patterns to hit on, files are file extensions to match on, and filters are negative patterns for any content pattern match.

A [definitions.toml](https://github.com/landaire/sourcesecrets/blob/master/definitions.toml) file useful for ASP.NET repositories has already been provided.

## Improvements to be made

1. The `git` utility is invoked for *every* commit to get contents and other details. Using some `libgit2` bindings or another library may provide benefits over the overhead of invoking a new process for every commit.
2. Add a "deleted-log" command that simply logs all files that were deleted

# Contributing Guidelines

Thank you for your interest in contributing to our project. Whether it's a bug report, new feature, correction, or additional
documentation, we greatly value feedback and contributions from our community.

Please read through this document before submitting any issues or pull requests to ensure we have all the necessary
information to effectively respond to your bug report or contribution.

## Reporting Bugs/Feature Requests

We welcome you to use the GitHub issue tracker to report bugs or suggest features.

When filing an issue, please check [existing open](https://github.com/awslabs/metrique/issues), or [recently closed](https://github.com/awslabs/metrique/issues?utf8=%E2%9C%93&q=is%3Aissue%20is%3Aclosed%20), issues to make sure somebody else hasn't already
reported the issue. Please try to include as much information as you can. Details like these are incredibly useful:

* A reproducible test case or series of steps
* The version of our code being used
* Any modifications you've made relevant to the bug
* Anything unusual about your environment or deployment

## Contributing via Pull Requests

Contributions via pull requests are much appreciated. Before sending us a pull request, please ensure that:

1. You are working against the latest source on the *main* branch.
2. You check existing open, and recently merged, pull requests to make sure someone else hasn't addressed the problem already.
3. You open an issue to discuss any significant work - we would hate for your time to be wasted.

To send us a pull request, please:

1. Fork the repository.
2. Modify the source; please focus on the specific change you are contributing. If you also reformat all the code, it will be hard for us to focus on your change.
3. Ensure local tests pass.
4. Commit to your fork using clear commit messages and ensure any Rust source files have been formatted with the [rustfmt tool](https://github.com/rust-lang/rustfmt#quick-start)
5. Send us a pull request, answering any default questions in the pull request interface.
6. Pay attention to any automated CI failures reported in the pull request, and stay involved in the conversation.

GitHub provides additional document on [forking a repository](https://help.github.com/articles/fork-a-repo/) and
[creating a pull request](https://help.github.com/articles/creating-a-pull-request/).

## Finding contributions to work on

Looking at the existing issues is a great way to find something to contribute on. As our projects, by default, use the default GitHub issue labels (enhancement/bug/duplicate/help wanted/invalid/question/wontfix), looking at any ['help wanted'](https://github.com/awslabs/metrique/labels/help%20wanted) issues is a great place to start.

## Dependencies on crates within the workspace

Dependencies on within-workspace crates within `[dependencies]` should be path-dependencies that
*include* a `version`, for example:

```toml
[dependencies]
...
metrique-writer-core = { path = "../metrique-writer-core", version = "0.1.4" }
```

If a dependency does not include a `version`, you will be unable to publish.

Dependencies within `[dev-dependencies]` should *not* include a `version`:

```toml
[dev-dependencies]
...
metrique = { path = "../metrique" }
```

If a dev-dependency *does* include a `version` and it goes the wrong way on the dependency
graph, you might encounter a chicken-and-egg problem when publishing, since `release-plz`
might update the `version` to the new version you are publishing.

## Doing releases

There is a `.github/workflows/release.yml` workflow that will attempt to use a crates.io release every time the version in the Cargo.toml changes. That is the sanctioned way of doing releases. The `release.yml` workflow is authorized to publish releases to the metrique crates via [trusted publishing], no further authorization is needed or desired for normal release publishing.

[trusted publishing]: https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html

To update the `Cargo.toml` and changelog, use [conventional commits], and in a clean git repo, run the following commands:
```
cargo install release-plz --locked
git checkout main && release-plz update
# before committing, make sure that CHANGELOG.md contains an appropriate changelog
git commit -a
```

Then make a new PR for the release and get it approved.

The automated release PR generation functionality is not used here.

[conventional commits]: https://www.conventionalcommits.org/en/v1.0.0/

### Publishing a new crate

trusted publishing is unable to publish new crates. If you want to add a new crate to the metrique family, you should:

1. create a branch that contains the crate you are publishing (it should
   be in the root `Cargo.toml`'s `workspace.members`, and in a publishable state).
2. add an entry to `release-plz.toml` of the form
   ```
   [[package]]
   name = "<package>"
   changelog_path = "CHANGELOG.md"
   ```
3. run `cargo publish -p <package> --dry-run`
4. get a temporary crates.io token just for the publishing
5. run `cargo login` with that token
6. run `cargo publish -p <package>`
7. set up trusted publishing via the crates.io WebUI to the following state:
   ```
   Publisher: Github
   Repository: awslabs/metrique
   Workflow: release.yml
   Environment: release
   ```
8. revoke the temporary crates.io token

Further publishing should happen via release-plz, without needing to manually work with tokens.

## External version (MSRV, dependency) policy

We reserve the right to increase the MSRV whenever new features make that useful or if
testing the current MSRV ever becomes technically difficult. However, the MSRV should
always at least be 8 weeks old.

The MSRV of a crate should always be newer than the MSRV of all of its dependencies to avoid
confusing Cargo's resolver. Currently for simplicity all metrique crates have the same MSRV,
and this is tested by `cargo-toml-format.rs`.

The code is tested by CI on the MSRV, on the current stable, and on the current nightly. In
fact, the testing of compiler error messages is only at the MSRV, since otherwise the
compiler message snapshots would have to be updated every 6 weeks.

For `crates.io` versions, tests are run against both the "current" crates.io commit and against
the lockfile checked into Github. The lockfile is normally updated on every release. Use with 
older versions of crates.io dependencies is possible but may not be supported (if
you encounter an issue that only happens with an old version of a dependency, our
suggested solution will probably be to upgrade it).

### Updating the MSRV

The `cargo-toml-format` test should assert that the MSRV is updated in all the
correct places, which are:

1. The `cargo-toml-format` test
2. The Cargo.toml of all packages
3. The UI tests
4. The test version in CI

## Code of Conduct

This project has adopted the [Amazon Open Source Code of Conduct](https://aws.github.io/code-of-conduct).
For more information see the [Code of Conduct FAQ](https://aws.github.io/code-of-conduct-faq) or contact
opensource-codeofconduct@amazon.com with any additional questions or comments.

## Security issue notifications

If you discover a potential security issue in this project we ask that you notify AWS/Amazon Security via our [vulnerability reporting page](http://aws.amazon.com/security/vulnerability-reporting/). Please do **not** create a public github issue.

## Licensing

See the [LICENSE](https://github.com/awslabs/metrique/blob/main/LICENSE) file for our project's licensing. We will ask you to confirm the licensing of your contribution.

We may ask you to sign a [Contributor License Agreement (CLA)](http://en.wikipedia.org/wiki/Contributor_License_Agreement) for larger changes.

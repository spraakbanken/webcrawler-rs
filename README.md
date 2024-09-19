# webcrawler-rs
Library for building a webcrawlers

[![MIT licensed][mit-badge]][mit-url]

[![Maturity badge - level 1][scorecard-badge]][scorecard-url]

[![CI(check)][actions-check-badge]][actions-check-url]
[![CI(scheduled)][actions-scheduled-badge]][actions-scheduled-url]
[![CI(test)][actions-test-badge]][actions-test-url]

[crates-badge]: https://img.shields.io/crates/v/mio.svg
[crates-url]: https://crates.io/crates/mio
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: LICENSE
[actions-check-badge]: https://github.com/spraakbanken/webcrawler-rs/actions/workflows/check.yml/badge.svg
[actions-check-url]: https://github.com/spraakbanken/webcrawler-rs/actions?query=workflow%3Acheck+branch%3Amain
[actions-scheduled-badge]: https://github.com/spraakbanken/webcrawler-rs/actions/workflows/scheduled.yml/badge.svg
[actions-scheduled-url]: https://github.com/spraakbanken/webcrawler-rs/actions?query=workflow%3Ascheduled+branch%3Amain
[actions-test-badge]: https://github.com/spraakbanken/webcrawler-rs/actions/workflows/test.yml/badge.svg
[actions-test-url]: https://github.com/spraakbanken/webcrawler-rs/actions?query=workflow%3Atest+branch%3Amain
[scorecard-badge]: https://img.shields.io/badge/Maturity-Level%201%20--%20New%20Project-yellow.svg
[scorecard-url]: https://github.com/spraakbanken/getting-started/blob/main/scorecard.md

## MSRV Policy

The MSRV (Minimum Supported Rust Version) is fixed for a given minor (1.x)
version. However it can be increased when bumping minor versions, i.e. going
from 1.0 to 1.1 allows us to increase the MSRV. Users unable to increase their
Rust version can use an older minor version instead. Below is a list of webcrawler versions
and their MSRV:

 * v0.1: Rust 1.70.

Note however that webcrawler also has dependencies, which might have different MSRV
policies. We try to stick to the above policy when updating dependencies, but
this is not always possible.

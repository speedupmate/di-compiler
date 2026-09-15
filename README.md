<p align="center">
  <img src="assets/fast-di-compile.svg" alt="Fast DI Compile logo" width="240">
</p>

<p align="center">
  <a href="https://github.com/speedupmate/di-compiler/actions/workflows/rust.yml?query=branch%3Amain">
    <img src="https://github.com/speedupmate/di-compiler/actions/workflows/rust.yml/badge.svg?branch=main" alt="Main build and test status">
  </a>
</p>

<h1 align="center">fast-di-compile</h1>

<p align="center">A battle-tested, production-ready replacement for Magento's <code>bin/magento setup:di:compile</code>.</p>

<p align="center"><strong>4 seconds instead of 50 seconds. 12.5× faster.</strong></p>

<details align="center">
<summary><strong>Time and money estimate</strong></summary>
<p align="left">
Each compilation estimatedly saves **46 seconds**. The table uses an 8-vCPU AWS CodeBuild Linux `general1.large` runner at **$0.02 per minute**, and assumes each build saves one billed minute. Developer time is valued at an assumed **$75 per hour**. The first two rows use 260 working days. The Magento 2 row uses 68 requested builds per calendar day, based on a public 30-day sample of the [Magento 2 testing workflow](https://github.com/magento/magento2/wiki/Magento-Automated-Testing), annualized to 365 days.

| Scenario | Builds per year | Time saved | Allocated CPU time | AWS build cost saved | Developer time value |
| --- | ---: | ---: | ---: | ---: | ---: |
| One daily production deployment | 260 | **3.3 hours** | 26.6 vCPU-hours | **$5.20** | **$249** |
| 10 developers, 20 test pipelines total per day | 5,200 | **66.4 hours** | 531.6 vCPU-hours | **$104** | **$4,983** |
| Magento 2 public CI activity, 68 builds per day | 24,820 | **317.1 hours** | 2,537.2 vCPU-hours | **$496** | **$23,786** |

- Estimate based on predicted price and time value, your milage may vary, faster process always wins.
</p>
</details>

## What it does

- Provides alternate and faster way to compile the code for Magento/Adobe Commerce
- Generates Magento's DI code and metadata.
- Processes files in parallel.
- Avoids rewriting unchanged files and reuses cached metadata.
- builds with RUST

## Get started

You will need a running magento installation:

- Rust and Cargo.
- A Magento project.

### 1. Installation

```bash
git clone https://github.com/speedupmate/di-compiler.git
cd di-compiler
cargo build --release -p fast-di-compile
```

The executable is created at `target/release/fast-di-compile`.

Rust and Cargo are needed only when building the compiler. The finished binary can be copied to a matching host and run without installing Rust.

#### Or build the binary with Docker

Alternatively you can build the binary in Docker and use the built binary on your host. Then you do not need Rust and Cargo on your magento host:

```bash
docker run --rm \
  --platform linux/amd64 \
  -v "$PWD:/src" \
  -w /src \
  rust:1.96-bookworm \
  cargo build --release -p fast-di-compile
```

The binary is written to `target/release/fast-di-compile` in the repository. Use a Docker platform that matches the host where the binary will run:

- `linux/amd64` for Intel and AMD Linux servers. This is the most common option.
- `linux/arm64` for ARM64 Linux servers, including AWS Graviton instances and Apple Silicon Mac.

For ARM64, change the platform in the command to `--platform linux/arm64`. The result is a Linux binary. 

A macOS or Windows host needs a native build for that operating system or must run the compiler inside Docker.

### 2. Compile your Magento project

Set `/path/to/magento` to your Magento root:

```bash
./target/release/fast-di-compile --magento-root /path/to/magento
```

By default the output goes to `generated/code` and `generated/metadata` in your Magento project.

### Useful options

| Option | What it does |
| --- | --- |
| `--output /path/to/output` | Set the output folder. Usually needed for comaprison in debuggin process |
| `--jobs 8` | Set parallel workers. Defaults to the CPU count. Sane value is your CPU count -4 |
| `--fallback-php /path/to/php` | Set the PHP executable. Defaults to `php`. |
| `--verbose` | Show detailed logs and timings. |
| `--help` | List all options. |

## How it works under the hood

The compiler is written in Rust. Rust scans Magento PHP files, reads `di.xml`, resolves dependencies, and writes generated code and metadata.

For Magento compatibility, it starts PHP worker processes when it needs runtime information. These workers load `<magento-root>/vendor/autoload.php` and use PHP reflection to inspect classes, constructors, constants, and methods. PHP also handles files the Rust parser cannot fully process.

When you run the binary on the host, it uses the host's PHP. Theoretically it is also possible to run it in Docker. But then the container must include a compatible PHP CLI runtime that matches host php setup. If this condition is met then you can mount the Magento directory into the container so the compiler can read the project and write generated files back to the host.

## Debugging

If your project has many third-party integrations and you face compilation issues, use the debugging tools below. Start with `--verbose` for detailed logs and timings.

The main way for debugging is to compare the output with Magento's compiler to investigate compatibility issues. The reports list missing, extra, and changed files.

<details>
<summary>Show comparison steps</summary>

### 1. Save Magento's output

Run Magento's compiler and copy its output to a new baseline folder:

```bash
cd /path/to/magento
bin/magento setup:di:compile
mkdir /tmp/magento-di-baseline
cp -R generated/code /tmp/magento-di-baseline/_code
cp -R generated/metadata /tmp/magento-di-baseline/_metadata
```

Refresh the baseline after changing code, dependencies, or configuration.

### 2. Generate and compare

From this repository's root directory:

```bash
./target/release/fast-di-compile \
  --magento-root /path/to/magento \
  --output /tmp/fast-di-output \
  --compare-archive \
  --archive-root /tmp/magento-di-baseline
```

Reports are in `/tmp/fast-di-output/diff/`:

- `summary.json`: totals.
- `.txt` files: missing, extra, and changed files.
- `comparable_metadata/*_report.txt`: detailed configuration differences.

Add `--compare-fail-on-diff` to fail the command when differences are found. File differences do not always mean a behavior change.

### 3. Point your AI agent to plans and ask it to debug the issues.

See the [project plan](.plans/README.md) and [ticket list](.plans/.tickets/README.md).

</details>

## Development

Source code is under `crates/`.

To check changes locally:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

## License and copyright

Copyright © 2026 Anton Siniorg.

Fast-Di-Compile is released under the [MIT License](LICENSE). Use it, improve it, and share it.

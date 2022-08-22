# Setting up a development environment

There are two ways to build the C2Rust project:

- [using **Docker**](../docker/)
- **manually**, as explained below:

The previous option automatically install all prerequisites during provisioning. You can also provision a macOS or Linux system manually.

- If you are on a Debian-based OS, you can run `./scripts/provision_deb.sh` to do so.

- If you are on macOS, install the Xcode command-line tools
(e.g., `xcode-select --install`) and [homebrew](https://brew.sh/) first.
Then run `./scripts/provision_mac.sh`.

- If you prefer to install dependencies yourself, or are using a non Debian-based Linux OS, our dependencies are as follows:
  - `cmake` >= 3.9.1
  - `dirmngr`
  - `curl`
  - `git`
  - `gnupg2`
  - `gperf`
  - `ninja`
  - `unzip`
  - `clang` >= 7
  - `intercept-build` or `bear` ([see why here](../README.md#generating-compilecommandsjson-files))
  - `python-dev`
  - `python` >= 3.6
  - [python dependencies](../scripts/requirements.txt)
  - `rustc` [version](../rust-toolchain.toml)
  - `rustfmt-preview` component for the above `rustc` version
  - `libssl` (development library, dependency of the refactoring tool)

## Building with system LLVM libraries

The quickest way to build the C2Rust transpiler
is with LLVM and clang system libraries (LLVM/clang >= 7 are currently supported).
If you have `libLLVM.so` and the `libclang` libraries (`libclangAST.a`, `libclangTooling.a`, etc. or their shared variants) installed,
you can build the transpiler with:

```sh
cd c2rust-transpile
cargo build
```

You can customize the location where the build system will look for LLVM using the following environment variables at compile time:

- `LLVM_CONFIG_PATH`: path (or filename if in `$PATH`) to the `llvm-config` tool of the LLVM installation
- `LLVM_LIB_DIR`: path to the `lib` directory of the LLVM installation (not necessary if you use `LLVM_CONFIG_PATH`)
- `LLVM_SYSTEM_LIBS`: additional system libraries LLVM needs to link against (e.g. `-lz -lrt -ldl`). Not necessary with `llvm-config`.
- `CLANG_PATH`: path to a clang that is the same version as your `libclang.so`.
  If this is necessary, the build system will return an error message explaining that.

C2Rust (indirectly) uses the [`clang-sys`](https://crates.io/crates/clang-sys) crate,
which can be configured with its own environment variables.

## Building dependencies from source

To develop on components that interact with LLVM,
we recommend building against a local copy of LLVM.
This will ensure that you have debug symbols and IDE integration for both LLVM and C2Rust.
However, building C2Rust from source with LLVM takes a while.
For a shorter build that links against prebuilt LLVM and clang system libraries,
you should be able to `cargo build` in the [`c2rust-transpile` directory](../c2rust-transpile/)
(see the general [README](../README.md)).

The following from-LLVM-source full build script
has been tested on recent versions of macOS and Ubuntu:

```sh
./scripts/build_translator.py
```

This downloads and builds LLVM under a new top-level folder named `build`.
Use the `C2RUST_BUILD_SUFFIX` variable to do multiple side-by-side builds
against a local copy of LLVM like this:

```sh
C2RUST_BUILD_SUFFIX=.debug ./scripts/build_translator.py --debug
```

*Note*: Set `C2RUST_BUILD_SUFFIX` if building inside and outside of the provided Docker environments from a single C2Rust checkout.

## Testing (Optional)

Tests are found in the [`tests`](../tests/) folder.
If you build the translator successfully, you should be able to run the tests with:

```sh
./scripts/test_translator.py tests
```

This basically tests that the original C file and translated Rust file
produce the same output when compiled and run.
More details about tests can be found in the [tests folder](../tests/).

*Note*: These run integration tests that invoke `c2rust transpile`.
`cargo test` only runs unit tests and doc tests as of now.

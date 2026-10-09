#!/usr/bin/env bash
# Decision 111: build the .NET P/Invoke model host for the *current* machine
# and copy it into a directory .NET probes (an app or test output directory),
# under the platform's native library name. Tests then load the identical
# Rust code that ships in the NuGet package's runtimes/<rid>/native/.
#
#   bash scripts/build_dotnet_host_native.sh <output-directory>
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${1:?usage: build_dotnet_host_native.sh <output-directory>}"

cargo build --quiet --locked --release -p traverse-dotnet-host --manifest-path "${repo_root}/Cargo.toml"
target_dir="$(cargo metadata --no-deps --format-version 1 --manifest-path "${repo_root}/Cargo.toml" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
case "$(uname -s)" in
  Darwin) library="libtraverse_dotnet_host.dylib" ;;
  MINGW* | MSYS* | CYGWIN*) library="traverse_dotnet_host.dll" ;;
  *) library="libtraverse_dotnet_host.so" ;;
esac
mkdir -p "${output}"
cp "${target_dir}/release/${library}" "${output}/${library}"
echo ".NET model host library: ${output}/${library}"

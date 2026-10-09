#!/usr/bin/env bash
# Decision 111 / #1643: install a packed TraverseEmbedder from a local feed into
# a throwaway console app and prove ExactModelHost loads the native model
# engine from the package's runtimes/<rid>/native/ (not from a build output).
#
#   bash scripts/ci/dotnet_nupkg_consumer_smoke.sh <directory-with-nupkg> <version>
set -euo pipefail

feed="$(cd "${1:?usage: dotnet_nupkg_consumer_smoke.sh <feed-dir> <version>}" && pwd)"
version="${2:?usage: dotnet_nupkg_consumer_smoke.sh <feed-dir> <version>}"
app="$(mktemp -d)"
trap 'rm -rf "${app}"' EXIT

cat >"${app}/Consumer.csproj" <<XML
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <RestoreSources>${feed};https://api.nuget.org/v3/index.json</RestoreSources>
    <RestorePackagesPath>${app}/packages</RestorePackagesPath>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="TraverseEmbedder" Version="${version}" />
  </ItemGroup>
</Project>
XML
cat >"${app}/Program.cs" <<'CS'
using Traverse.Embedder;

// Creating a host and querying it runs two framed calls through the packaged
// native library; a missing library throws engine_unavailable instead.
using var host = new ExactModelHost([], [], "commercial");
Console.WriteLine(host.ModelRights("00") is null ? "native-model-host-ok" : "unexpected-rights");
CS

if ! output="$(dotnet run --project "${app}/Consumer.csproj" 2>&1)"; then
  echo "${output}" >&2
  echo "the consumer app failed to build or run" >&2
  exit 1
fi
echo "${output}"
if ! grep -Fxq "native-model-host-ok" <<<"${output}"; then
  echo "the packaged native model host did not load" >&2
  exit 1
fi

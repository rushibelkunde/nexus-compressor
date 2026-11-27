#!/usr/bin/env bash
# Downloads the benchmark corpus and reference tools into ./bench-corpus
# (git-ignored). Run from the repository root:
#
#     bash scripts/bench/fetch_corpus.sh
#
# What it fetches:
#   * the Silesia corpus (12 files, 202 MB uncompressed) - the standard
#     mixed-content benchmark for general-purpose compressors;
#   * on Windows, a zstd release binary to compare against (other reference
#     tools - gzip, bzip2, xz, brotli - are picked up from PATH or from a
#     Git for Windows installation if present).
set -euo pipefail

SILESIA_URL="https://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip"
ZSTD_VERSION="1.5.6"
ZSTD_URL="https://github.com/facebook/zstd/releases/download/v${ZSTD_VERSION}/zstd-v${ZSTD_VERSION}-win64.zip"

mkdir -p bench-corpus/silesia bench-corpus/tools

if [ ! -f bench-corpus/silesia/dickens ]; then
    echo "Downloading Silesia corpus..."
    curl -fL --retry 3 -o bench-corpus/silesia.zip "$SILESIA_URL"
    unzip -q -o bench-corpus/silesia.zip -d bench-corpus/silesia
    rm -f bench-corpus/silesia.zip
else
    echo "Silesia corpus already present."
fi

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
        if ! ls bench-corpus/tools/*/zstd.exe >/dev/null 2>&1; then
            echo "Downloading zstd ${ZSTD_VERSION} for Windows..."
            curl -fL --retry 3 -o bench-corpus/tools/zstd.zip "$ZSTD_URL"
            unzip -q -o bench-corpus/tools/zstd.zip -d bench-corpus/tools
            rm -f bench-corpus/tools/zstd.zip
        else
            echo "zstd already present."
        fi
        ;;
    *)
        echo "Install zstd, xz, brotli and bzip2 with your package manager to compare against them."
        ;;
esac

echo "Done. Files:"
ls -l bench-corpus/silesia

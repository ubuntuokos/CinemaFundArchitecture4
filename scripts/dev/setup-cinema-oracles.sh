#!/usr/bin/env bash
# External DCP / IMF test oracles for CFA4 (CINECERT WP01-A; plan docs/plans/CFA4-CINECERT-001-v2.3.md
# chapter 13, decision D1 = A: these tools are test oracles only, never part of a CFA4 package).
#
#   asdcplib 2.13.2  (CineCert, BSD-3-Clause)  asdcp-info, asdcp-unwrap, as-02-* … built from the
#                                               pinned commit into its own prefix (CFA4_ASDCPLIB_PREFIX)
#   Photon 5.0.1     (Netflix, Apache-2.0)      IMF validator: the released jar and its runtime
#                                               dependencies from Maven Central, sha256-checked (CFA4_PHOTON_CP)
#   Clairmeta 1.6.3  (Ymagis, BSD-3-Clause)     DCP checker, in its own venv from the hash-pinned
#                                               requirements/clairmeta-oracle.txt (CFA4_CLAIRMETA)
#   corpus                                      external DCP / IMF test packages listed in
#                                               scripts/dev/cinema-oracles/corpus.txt, sha256-checked
#
#   --check                (default) read-only: report prerequisites and what is installed
#   --install [WHAT...]    install asdcplib, photon, clairmeta (default: all three)
#   --corpus               fetch every corpus entry that is missing or has the wrong sha256
#   --print-env            print the variables the CFA4 tests read (shell-sourceable)
#
# Environment:
#   CFA4_CINEMA_ORACLES_DIR  cache root (default: ${XDG_CACHE_HOME:-~/.cache}/cfa4/cinema-oracles)
#   CFA4_MAVEN_CENTRAL       Maven Central base URL (default: https://repo1.maven.org/maven2)
#
# Nothing is installed into the system, the user's Python, Java or Maven setup: everything goes
# under the cache root, and missing system tools are reported with the apt command to install
# them (the script never runs apt or sudo). Remove the cache root to uninstall.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
# Reject non-Ubuntu and non-26.04 hosts, as the other developer scripts do.
. "${here}/../ci/require-ubuntu-2604.sh"

ASDCPLIB_URL="https://github.com/cinecert/asdcplib"
ASDCPLIB_VERSION="2.13.2"
ASDCPLIB_COMMIT="d53aac87d1e52d71bbc47631911e75e01e6ad328" # tag rel_2_13_2
PHOTON_VERSION="5.0.1"
PHOTON_GAV="com.netflix.photon:Photon:${PHOTON_VERSION}"
PHOTON_LOCK="${here}/cinema-oracles/photon-${PHOTON_VERSION}.sha256"
MAVEN_DEPENDENCY_PLUGIN="org.apache.maven.plugins:maven-dependency-plugin:3.8.1"
CLAIRMETA_VERSION="1.6.3"
CLAIRMETA_LOCK="${repo}/requirements/clairmeta-oracle.txt"
CORPUS_LIST="${here}/cinema-oracles/corpus.txt"
MAVEN_CENTRAL="${CFA4_MAVEN_CENTRAL:-https://repo1.maven.org/maven2}"

root="${CFA4_CINEMA_ORACLES_DIR:-${XDG_CACHE_HOME:-${HOME}/.cache}/cfa4/cinema-oracles}"
asdcp_dir="${root}/asdcplib-${ASDCPLIB_VERSION}"
asdcp_prefix="${asdcp_dir}/prefix"
photon_dir="${root}/photon-${PHOTON_VERSION}"
clairmeta_dir="${root}/clairmeta-${CLAIRMETA_VERSION}"
clairmeta_wrapper="${clairmeta_dir}/bin/cfa4-clairmeta"
corpus_dir="${root}/corpus"

status=0
ok() { printf 'OK      %s\n' "$*"; }
miss() { printf 'MISSING %s\n' "$*"; status=1; }
note() { printf 'NOTE    %s\n' "$*"; }
die() { echo "setup-cinema-oracles: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required: $2"; }

# Exit code 0 when the photon lib directory holds exactly the locked jars with the locked hashes.
photon_ok() {
  [[ -d "${photon_dir}/lib" ]] || return 1
  (cd "${photon_dir}/lib" && sha256sum --check --quiet --strict "$PHOTON_LOCK" >/dev/null 2>&1) || return 1
  [[ "$(find "${photon_dir}/lib" -maxdepth 1 -name '*.jar' | wc -l)" == "$(grep -c . "$PHOTON_LOCK")" ]]
}

check() {
  local t
  for t in git cmake g++ sha256sum curl; do
    if command -v "$t" >/dev/null 2>&1; then ok "$t"; else miss "$t: sudo apt install git cmake g++ coreutils curl"; fi
  done
  if [[ -r /usr/include/openssl/rand.h ]]; then ok "OpenSSL headers (asdcplib encryption support)"; else
    miss "OpenSSL headers: sudo apt install libssl-dev"
  fi
  if [[ -r /usr/include/xercesc/dom/DOM.hpp ]]; then ok "Xerces-C headers (asdcplib XML support)"; else
    note "Xerces-C headers not found (optional; asdcplib then builds without XML parse support): sudo apt install libxerces-c-dev"
  fi
  if command -v java >/dev/null 2>&1; then ok "java ($(java -version 2>&1 | awk -F'"' '/version/ {print $2; exit}'))"; else
    miss "java >= 17 for Photon: sudo apt install openjdk-21-jre-headless"
  fi
  if command -v mvn >/dev/null 2>&1; then ok "mvn (fetches Photon)"; else miss "mvn for --install photon: sudo apt install maven"; fi
  if python3 -c 'import venv, ensurepip' >/dev/null 2>&1; then ok "python3 venv"; else
    miss "python3 venv for Clairmeta: sudo apt install python3-venv"
  fi
  for t in mediainfo sox; do
    if command -v "$t" >/dev/null 2>&1; then ok "$t (Clairmeta, optional)"; else note "$t not found (optional for Clairmeta): sudo apt install $t"; fi
  done
  if [[ -x "${asdcp_prefix}/bin/asdcp-info" ]]; then
    ok "$("${asdcp_prefix}/bin/asdcp-info" -V 2>/dev/null | grep -o -m1 'asdcplib [0-9.]*') at ${asdcp_prefix}"
  else
    miss "asdcplib ${ASDCPLIB_VERSION}: bash scripts/dev/setup-cinema-oracles.sh --install asdcplib"
  fi
  if photon_ok; then ok "Photon ${PHOTON_VERSION} at ${photon_dir}/lib (sha256 lock matches)"; else
    miss "Photon ${PHOTON_VERSION}: bash scripts/dev/setup-cinema-oracles.sh --install photon"
  fi
  if [[ -x "$clairmeta_wrapper" ]] && "${clairmeta_dir}/venv/bin/python" -c "import clairmeta, sys; sys.exit(clairmeta.__version__ != '${CLAIRMETA_VERSION}')" 2>/dev/null; then
    ok "Clairmeta ${CLAIRMETA_VERSION} (${clairmeta_wrapper})"
  else
    miss "Clairmeta ${CLAIRMETA_VERSION}: bash scripts/dev/setup-cinema-oracles.sh --install clairmeta"
  fi
  local entries
  entries="$(corpus_entries | wc -l)"
  if [[ "$entries" == 0 ]]; then note "corpus: no entries in ${CORPUS_LIST#"${repo}"/}"; else
    local have=0 name url sum
    while read -r name url sum; do
      [[ -f "${corpus_dir}/${name}" ]] && have=$((have + 1))
    done < <(corpus_entries)
    if [[ "$have" == "$entries" ]]; then ok "corpus: ${have} of ${entries} in ${corpus_dir}"; else
      miss "corpus: ${have} of ${entries} in ${corpus_dir}: bash scripts/dev/setup-cinema-oracles.sh --corpus"
    fi
  fi
  return "$status"
}

install_asdcplib() {
  need git "sudo apt install git"
  need cmake "sudo apt install cmake"
  need g++ "sudo apt install g++"
  [[ -r /usr/include/openssl/rand.h ]] || die "OpenSSL headers are required: sudo apt install libssl-dev"
  local src="${asdcp_dir}/src" build="${asdcp_dir}/build"
  mkdir -p "$asdcp_dir"
  if [[ ! -d "${src}/.git" ]]; then
    git init --quiet "$src"
    git -C "$src" remote add origin "$ASDCPLIB_URL"
  fi
  if [[ "$(git -C "$src" rev-parse HEAD 2>/dev/null || true)" != "$ASDCPLIB_COMMIT" ]]; then
    git -C "$src" fetch --quiet --depth 1 origin "$ASDCPLIB_COMMIT"
    git -C "$src" checkout --quiet --detach FETCH_HEAD
  fi
  [[ "$(git -C "$src" rev-parse HEAD)" == "$ASDCPLIB_COMMIT" ]] || die "asdcplib checkout is not ${ASDCPLIB_COMMIT}"
  [[ -z "$(git -C "$src" status --porcelain)" ]] || die "asdcplib checkout at ${src} has local changes; remove ${asdcp_dir} and run again"
  rm -rf "$build" "$asdcp_prefix"
  # The binaries find their shared libraries through RPATH, so nothing goes on LD_LIBRARY_PATH.
  local log="${asdcp_dir}/build.log"
  # shellcheck disable=SC2016 # $ORIGIN is for the dynamic linker, not the shell
  if ! {
    cmake -S "$src" -B "$build" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$asdcp_prefix" \
      -DCMAKE_INSTALL_RPATH='$ORIGIN/../lib' &&
      cmake --build "$build" -j "$(nproc)" &&
      cmake --install "$build"
  } >"$log" 2>&1; then
    tail -n 40 "$log" >&2
    die "asdcplib build failed (full log: ${log})"
  fi
  "${asdcp_prefix}/bin/asdcp-info" -V | grep -q "asdcplib ${ASDCPLIB_VERSION}" || die "asdcp-info does not report ${ASDCPLIB_VERSION}"
  echo "Installed asdcplib ${ASDCPLIB_VERSION} (${ASDCPLIB_COMMIT}) under ${asdcp_prefix}"
}

install_photon() {
  need java "sudo apt install openjdk-21-jre-headless"
  need mvn "sudo apt install maven"
  [[ -f "$PHOTON_LOCK" ]] || die "missing lock file ${PHOTON_LOCK}"
  local work="${photon_dir}/maven"
  mkdir -p "$work"
  # A throwaway project whose only dependency is the Photon release; Maven resolves its runtime
  # dependencies from Maven Central into a private local repository.
  cat > "${work}/pom.xml" <<EOF
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>cfa4.oracle</groupId><artifactId>photon-oracle</artifactId><version>1</version><packaging>pom</packaging>
  <dependencies>
    <dependency><groupId>${PHOTON_GAV%%:*}</groupId><artifactId>Photon</artifactId><version>${PHOTON_VERSION}</version></dependency>
  </dependencies>
</project>
EOF
  cat > "${work}/settings.xml" <<EOF
<settings><mirrors><mirror><id>cfa4-central</id><mirrorOf>*</mirrorOf><url>${MAVEN_CENTRAL}</url></mirror></mirrors></settings>
EOF
  rm -rf "${photon_dir:?}/lib"
  (cd "$work" && mvn -q -B -s settings.xml -Dmaven.repo.local="${work}/repository" \
    "${MAVEN_DEPENDENCY_PLUGIN}:copy-dependencies" -DincludeScope=runtime -DoutputDirectory="${photon_dir}/lib")
  if ! photon_ok; then
    (cd "${photon_dir}/lib" && sha256sum --check --strict "$PHOTON_LOCK") || true
    die "Photon jars do not match ${PHOTON_LOCK#"${repo}"/}; nothing is used until they do"
  fi
  echo "Installed Photon ${PHOTON_VERSION} under ${photon_dir}/lib"
}

install_clairmeta() {
  python3 -c 'import venv, ensurepip' >/dev/null 2>&1 || die "python3 venv is required: sudo apt install python3-venv"
  [[ -f "$CLAIRMETA_LOCK" ]] || die "missing lock file ${CLAIRMETA_LOCK}"
  rm -rf "${clairmeta_dir}/venv"
  mkdir -p "${clairmeta_dir}/bin"
  python3 -m venv "${clairmeta_dir}/venv"
  "${clairmeta_dir}/venv/bin/python" -m pip install --quiet --disable-pip-version-check --no-input \
    --require-hashes --only-binary=:all: -r "$CLAIRMETA_LOCK"
  # Clairmeta calls asdcp-info and asdcp-unwrap from PATH: the wrapper puts our asdcplib prefix first.
  cat > "$clairmeta_wrapper" <<EOF
#!/usr/bin/env bash
# Generated by scripts/dev/setup-cinema-oracles.sh: Clairmeta ${CLAIRMETA_VERSION} in its own venv,
# asdcplib ${ASDCPLIB_VERSION} first on PATH. Usage: cfa4-clairmeta check -type dcp /path/to/DCP
set -euo pipefail
p="\${PATH}"
[[ -d "${asdcp_prefix}/bin" ]] && p="${asdcp_prefix}/bin:\${p}"
exec env -u PYTHONPATH -u PYTHONHOME PATH="\${p}" "${clairmeta_dir}/venv/bin/python" -m clairmeta.cli "\$@"
EOF
  chmod 0755 "$clairmeta_wrapper"
  echo "Installed Clairmeta ${CLAIRMETA_VERSION}: ${clairmeta_wrapper}"
}

# "name url sha256" lines of the corpus list, without comments and blank lines.
corpus_entries() {
  [[ -f "$CORPUS_LIST" ]] || return 0
  grep -v -E '^[[:space:]]*(#|$)' "$CORPUS_LIST" || true
}

corpus() {
  need curl "sudo apt install curl"
  mkdir -p "$corpus_dir"
  local name url sum n=0
  while read -r name url sum; do
    [[ -n "$name" && -n "$url" && "$sum" =~ ^[0-9a-f]{64}$ ]] || die "bad corpus line: ${name} ${url} ${sum}"
    [[ "$name" =~ ^[A-Za-z0-9._-]+$ ]] || die "corpus name must be a plain file name: ${name}"
    [[ "$url" == https://* ]] || die "corpus URL must be https: ${url}"
    local dst="${corpus_dir}/${name}"
    if [[ -f "$dst" ]] && echo "${sum}  ${dst}" | sha256sum --check --quiet --strict >/dev/null 2>&1; then
      n=$((n + 1)); continue
    fi
    curl --fail --location --silent --show-error --proto '=https' --output "${dst}.part" "$url"
    if ! echo "${sum}  ${dst}.part" | sha256sum --check --quiet --strict; then
      rm -f "${dst}.part"
      die "sha256 mismatch for ${name} (${url})"
    fi
    mv "${dst}.part" "$dst"
    n=$((n + 1))
  done < <(corpus_entries)
  echo "Corpus: ${n} file(s) in ${corpus_dir}"
}

print_env() {
  printf 'CFA4_ASDCPLIB_PREFIX=%q\n' "$asdcp_prefix"
  printf 'CFA4_PHOTON_CP=%q\n' "${photon_dir}/lib/*"
  printf 'CFA4_CLAIRMETA=%q\n' "$clairmeta_wrapper"
  printf 'CFA4_CINEMA_CORPUS=%q\n' "$corpus_dir"
}

mode="${1:---check}"
[[ $# -gt 0 ]] && shift
case "$mode" in
  --check) check ;;
  --install)
    what=("$@")
    [[ ${#what[@]} -gt 0 ]] || what=(asdcplib photon clairmeta)
    for w in "${what[@]}"; do
      case "$w" in
        asdcplib) install_asdcplib ;;
        photon) install_photon ;;
        clairmeta) install_clairmeta ;;
        *) die "unknown oracle ${w} (asdcplib, photon, clairmeta)" ;;
      esac
    done
    ;;
  --corpus) corpus ;;
  --print-env) print_env ;;
  *)
    echo "Usage: bash scripts/dev/setup-cinema-oracles.sh [--check|--install [asdcplib|photon|clairmeta]...|--corpus|--print-env]" >&2
    exit 2
    ;;
esac

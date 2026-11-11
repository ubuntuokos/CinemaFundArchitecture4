#!/usr/bin/env bash
# R3D-REDLINE: unpack a user-supplied RED REDline Linux installer into a private,
# self-contained directory WITHOUT root and WITHOUT running the vendor installer.
#
# The REDline installer is a Makeself archive whose embedded script copies its
# binary and libraries into /usr/local and runs ldconfig as root. This script
# only reads it: it checks the Makeself MD5, lists the archive, refuses unsafe
# member paths, extracts it into an empty temporary directory and lays out
#
#   <dest>/libexec/REDline + the bundled .so files (REDline looks for
#                             REDR3D-x64.so next to its own binary)
#   <dest>/redline          wrapper that sets LD_LIBRARY_PATH for the bundled Qt 5
#   <dest>/VERSION          REDline build and R3D SDK version it reports
#
# REDline is proprietary RED software: never commit it, the installer or the
# unpacked files to any repository (docs/development/redline.md).
#
# Usage: scripts/dev/redline-unpack.sh <REDline_Build_*_Installer.sh> [dest]
#   dest defaults to ~/.local/opt/redline; it must not exist or be empty.
#   Afterwards: export CFA4_REDLINE=<dest>/redline
set -euo pipefail

die() { echo "redline-unpack: $*" >&2; exit 2; }

[[ $# -ge 1 && $# -le 2 ]] || die "usage: $0 <REDline installer .sh> [dest]"
INSTALLER="$1"
DEST="${2:-$HOME/.local/opt/redline}"
[[ -f "$INSTALLER" ]] || die "installer not found: $INSTALLER"
for tool in md5sum gzip tar head tail grep sed; do
    command -v "$tool" >/dev/null || die "$tool is required"
done
if [[ -e "$DEST" ]] && [[ -n "$(ls -A "$DEST" 2>/dev/null)" ]]; then
    die "destination is not empty: $DEST"
fi

# --- read the Makeself header (text only, never sourced or executed) ----------
HEADER="$(head -c 16384 "$INSTALLER" | tr -d '\000')"
grep -q '^# This script was generated using Makeself' <<<"$HEADER" \
    || die "not a Makeself archive: $INSTALLER"
field() { sed -n "s/^$1=\"\([^\"]*\)\"\$/\1/p" <<<"$HEADER" | head -n 1; }
LABEL="$(field label)"
MD5="$(field MD5)"
SIZES="$(field filesizes)"
# shellcheck disable=SC2016 # the backticks and $0 are literal header text
LINES="$(sed -n 's/^offset=`head -n \([0-9][0-9]*\) "\$0".*/\1/p' <<<"$HEADER" | head -n 1)"
[[ "$LABEL" == REDline* ]] || die "unexpected archive label '$LABEL' (expected REDline …)"
[[ "$SIZES" =~ ^[0-9]+$ ]] || die "expected exactly one embedded archive, got filesizes='$SIZES'"
[[ "$MD5" =~ ^[0-9a-f]{32}$ ]] || die "missing embedded MD5"
[[ "$LINES" =~ ^[0-9]+$ ]] || die "cannot find the payload offset"
OFFSET="$(head -n "$LINES" "$INSTALLER" | wc -c | tr -d ' ')"
payload() { tail -c +"$((OFFSET + 1))" "$INSTALLER" | head -c "$SIZES"; }

GOT="$(payload | md5sum | cut -c1-32)"
[[ "$GOT" == "$MD5" ]] || die "MD5 mismatch: header $MD5, payload $GOT (corrupt download?)"
echo "OK  $LABEL, payload MD5 $MD5"

# --- list before extracting: only relative paths, no '..', no links out -------
while IFS= read -r name; do
    case "$name" in
        /*|../*|*/../*|*/..|..) die "unsafe path in archive: $name" ;;
    esac
done < <(payload | gzip -cd | tar -tf -)
LISTING="$(payload | gzip -cd | tar -tvf -)"
while IFS= read -r line; do
    if [[ "$line" == l* ]]; then
        target="${line##* -> }"
        case "$target" in
            /*|*..*) die "unsafe symlink in archive: $name -> $target" ;;
        esac
    fi
    [[ "$line" == [-dl]* ]] || die "unexpected member type in archive: $line"
done <<<"$LISTING"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
payload | gzip -cd | tar -xf - -C "$WORK" --no-same-owner --no-same-permissions
for f in bin/REDline libs/REDR3D-x64.so libs/REDDecoder-x64.so; do
    [[ -f "$WORK/$f" ]] || die "archive lacks $f"
done

# --- private layout ------------------------------------------------------------
mkdir -p "$DEST/libexec"
cp -d "$WORK"/libs/* "$DEST/libexec/"
install -m 0755 "$WORK/bin/REDline" "$DEST/libexec/REDline"
# The installer relies on ldconfig for the Qt 5 soname links; create them here.
(
    cd "$DEST/libexec"
    for f in lib*.so.*.*.*; do
        [[ -e "$f" ]] || continue
        soname="$(sed -E 's/^(.*\.so\.[0-9]+)\..*$/\1/' <<<"$f")"
        [[ -e "$soname" ]] || ln -s "$f" "$soname"
    done
)
cat >"$DEST/redline" <<'EOF'
#!/usr/bin/env bash
# Wrapper written by CFA4 scripts/dev/redline-unpack.sh: runs the private REDline copy.
HERE="$(cd -- "$(dirname -- "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
export LD_LIBRARY_PATH="$HERE/libexec${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$HERE/libexec/REDline" "$@"
EOF
chmod 0755 "$DEST/redline"

# --- smoke test: REDline prints its build and R3D SDK version on --help --------
HELP="$("$DEST/redline" --help 2>&1 || true)"
BUILD="$(grep -o 'REDline Build [0-9.]*' <<<"$HELP" | head -n 1 || true)"
SDK="$(grep -o 'R3DSDK [0-9][0-9.]*-[0-9a-f]*' <<<"$HELP" | head -n 1 || true)"
if [[ -z "$BUILD" ]]; then
    echo "$HELP" | tail -n 5 >&2
    die "the unpacked REDline does not start (missing system libraries? see docs/development/redline.md)"
fi
{
    echo "installer: $(basename -- "$INSTALLER")"
    echo "installer-md5: $MD5"
    echo "redline: $BUILD"
    echo "r3dsdk: ${SDK:-unknown}"
} >"$DEST/VERSION"
cat "$DEST/VERSION"
echo
echo "Unpacked to $DEST. Use it with:"
echo "  export CFA4_REDLINE=$DEST/redline"

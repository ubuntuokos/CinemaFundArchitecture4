#!/usr/bin/env bash
set -eo pipefail

version='2.13.2'
python_bin='python3'

if [[ -n "${CFA4_SKILLSPECTOR_HOME-}" ]]; then
    tool_home="$CFA4_SKILLSPECTOR_HOME"
else
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    tool_home="$data_home/cfa4/tools"
fi

venv="$tool_home/skillspector/$version/venv"
cli="$venv/bin/skillspector"

if ! "$python_bin" -c 'import sys; raise SystemExit(0 if (3, 12) <= sys.version_info[:2] < (3, 15) else 1)'; then
    printf '%s\n' 'SkillSpector 2.13.2 requires Python 3.12, 3.13, or 3.14.' >&2
    exit 2
fi

if [[ -x "$cli" ]]; then
    version_output=$("$cli" --version)
    if [[ "$version_output" == *"$version"* ]]; then
        printf '%s\n' "$version_output"
        exit 0
    fi
    printf '%s\n' "Refusing to replace an existing CLI at $cli." >&2
    exit 1
fi

if [[ -e "$venv" ]]; then
    printf '%s\n' "An incomplete or unexpected venv exists at $venv; refusing to modify it." >&2
    exit 1
fi

mkdir -p "$(dirname "$venv")"
"$python_bin" -m venv "$venv"
"$venv/bin/python" -m pip install --disable-pip-version-check 'skillspector==2.13.2'
"$venv/bin/python" -m pip check
version_output=$("$cli" --version)
if [[ "$version_output" != *"$version"* ]]; then
    printf '%s\n' "Installed CLI version does not match pinned version $version: $version_output" >&2
    exit 1
fi
printf '%s\n' "$version_output"

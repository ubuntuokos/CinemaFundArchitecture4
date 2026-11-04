#!/usr/bin/env bash
set -eo pipefail

version='2.13.2'
if [[ -n "${CFA4_SKILLSPECTOR_HOME-}" ]]; then
    tool_home="$CFA4_SKILLSPECTOR_HOME"
else
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    tool_home="$data_home/cfa4/tools"
fi

cli="$tool_home/skillspector/$version/venv/bin/skillspector"
if [[ ! -x "$cli" ]]; then
    printf '%s\n' 'SkillSpector is not installed in the CFA4 isolated tool environment.' >&2
    printf '%s\n' 'Install it explicitly with: bash scripts/dev/install-skillspector.sh' >&2
    exit 127
fi

exec "$cli" "$@"

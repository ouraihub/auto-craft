# Fails when a tracked file names the framework this repository was forked from (docs/02 §2.3).
# The name is assembled here so this script does not match itself.
$ErrorActionPreference = 'Stop'
$name = 'think' + 'rail'
$self = 'scripts/check-names.ps1'
$root = git rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0) { exit 2 }
Push-Location $root
try {
    $hits = git grep -n -I -i -e $name -- . ":(exclude)$self"
    $status = $LASTEXITCODE
    if ($status -eq 0) {
 $hits | ForEach-Object { Write-Output $_ }
        Write-Error "Found the framework source's product name in tracked files."
        exit 1
    }
    if ($status -ne 1) { exit $status }
    Write-Output 'check-names: ok'
    exit 0
} finally {
    Pop-Location
}

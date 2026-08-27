# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: run.ps1
#  script_path: run.ps1
#  module_name: run
#  version: 0.1.0
#  description: Windows build-and-launch driver for the Godot shell.
#           Mirrors run.sh flag for flag so a Linux-authored command line
#           works unchanged here; the three platform splits
#           (metrum_rise.dll instead of the .so, mods under APPDATA, Godot
#           found via -Godot, GODOT, or PATH) are the only divergence.
#           Carries the debug category switches and the Godot import-cache
#           repair pass because those are launch-time environment concerns,
#           not engine code.
#  kind: module
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [godot, cargo]
#  features: [build-script, launcher, debug-flags, windows]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-24
# =========================================================================

# Metrum Rise Run Script (Windows)
#
# PowerShell equivalent of run.sh. Same flags, same environment variables, same
# behaviour, with three platform differences:
#   - builds and deploys metrum_rise.dll instead of libmetrum_rise.so
#   - user assets live under %APPDATA%\Godot\app_userdata\Metrum Rise\mods\
#   - Godot is located via -Godot, $env:GODOT, or PATH
#
# Debug modes (identical to run.sh):
#   --debug              General debug logging (stdout)
#   --debug <category>   Category-filtered debug logging
#                        Categories: isect, economy, demand, spawn, road, border,
#                        terrain, buildings, visuals, perf, traffic, world-editor
#   --debug road         Road placement timings and geometry dumps
#   --debug terrain      Terrain + water patch residency/perf summaries
#   --debug terrain-verbose / terrain-full / terrain-lod1 / terrain-full-lod1
#   --debug terrain-visual <mode>
#                        Modes: patch, lod, height, relief, shore, water-depth,
#                        water-lod, water-patch, water-material, lighting
#   --debug perf         Frame CPU diagnostics by renderer
#   --debug buildings    Building-site mesh/material diagnostics
#   --debug building-sites-visual [mode]
#   --debug site-grading Road geometry + building-site diagnostics + overlay
#   --debug traffic      Traffic/routing + road-network connectivity (stderr)
#   --debug-traffic      Alias for --debug traffic
#   --pedestrian-vat-debug <mode>    Modes: rest, uv, off
#   --debug-world-editor Alias for --debug world-editor
#   --debug-sim          Hourly simulation summaries
#   --debug visuals [mode] / --visuals [mode]
#                        Modes: raw, macro, mid, micro, fades, material, height,
#                        mask, luminance, footprint
#
# Release crash diagnostics:
#   --release defaults METRUM_CRASH_DIAGNOSTICS=1 and writes panic dumps to logs\
#   $env:METRUM_CRASH_DIAGNOSTICS=0 disables the background recorder
#
# Godot import cache:
#   $env:METRUM_SKIP_GODOT_IMPORT_REPAIR=1 skips the repair pass.
#   $env:METRUM_FORCE_GODOT_IMPORT_REPAIR=1 retries after a previous attempt.
#
# Usage:
#   .\run.ps1 --release
#   .\run.ps1 --release --asset-editor
#   .\run.ps1 --debug traffic
#   .\run.ps1 --headless --quit-after 300

[CmdletBinding()]
param(
    # Explicit Godot path. Falls back to $env:GODOT then PATH.
    # Named only, for example: -Godot <full path to the godot exe>
    [Parameter(Mandatory = $false)]
    [string]$Godot = "",

    # Every other argument is passed through to the flag parser below.
    [Parameter(ValueFromRemainingArguments = $true, Position = 0)]
    [string[]]$Rest = @()
)

# $Args is automatic in PowerShell; use our own name for the parsed list.
$ArgList = @($Rest)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$GodotDir    = Join-Path $ProjectRoot "godot"
$RustDir     = Join-Path $ProjectRoot "rust"

# ---------------------------------------------------------------- flag state
$Release                     = $false
$DebugOn                     = $false
$DebugCategory               = ""
$DebugTraffic                = $false
$DebugSim                    = $false
$DebugPerf                   = $false
$DebugBuildings              = $false
$DebugSurface                = $false
$DebugRoadGeometryDump       = $false
$VisualDebug                 = $false
$VisualDebugMode             = "material"
$TerrainDebug                = $false
$TerrainVerbose              = $false
$TerrainForceFullWorld       = $false
$TerrainForceLod             = ""
$TerrainVisualMode           = ""
$TerrainGrass                = $false
$BuildingSiteVisualDebug     = $false
$BuildingSiteVisualDebugMode = "material"
$PedestrianVatDebugMode      = ""
$GodotEngineArgs             = New-Object System.Collections.Generic.List[string]
$GodotArgs                   = New-Object System.Collections.Generic.List[string]

function Set-TerrainPreset {
    param([string]$Mode)
    $script:TerrainDebug = $true
    switch ($Mode) {
        "terrain-verbose"   { $script:TerrainVerbose = $true }
        "terrain-full"      { $script:TerrainForceFullWorld = $true }
        "terrain-lod1"      { $script:TerrainForceLod = "1" }
        "terrain-full-lod1" { $script:TerrainForceFullWorld = $true; $script:TerrainForceLod = "1" }
    }
}

# ------------------------------------------------------------------ parsing
$i = 0
while ($i -lt $ArgList.Count) {
    $arg = $ArgList[$i]
    $next = if ($i + 1 -lt $ArgList.Count) { $ArgList[$i + 1] } else { $null }
    $nextIsValue = ($null -ne $next) -and (-not $next.StartsWith("--"))

    switch -Regex ($arg) {
        '^--release$'  { $Release = $true }
        '^--headless$' { $GodotEngineArgs.Add($arg) }

        '^--quit-after$' {
            $GodotEngineArgs.Add($arg)
            if ($null -ne $next) { $GodotEngineArgs.Add($next); $i++ }
        }
        '^--quit-after=' { $GodotEngineArgs.Add($arg) }

        '^--debug-sim$'          { $DebugSim = $true }
        '^--debug-traffic$'      { $DebugTraffic = $true }
        '^--debug-world-editor$' { $DebugOn = $true; $DebugCategory = "world-editor" }

        '^--visuals=' {
            $VisualDebug = $true
            $VisualDebugMode = $arg.Substring("--visuals=".Length)
        }
        '^--visuals$' {
            $VisualDebug = $true
            if ($nextIsValue) { $VisualDebugMode = $next; $i++ }
        }

        '^--pedestrian-vat-debug=' {
            $PedestrianVatDebugMode = $arg.Substring("--pedestrian-vat-debug=".Length)
        }
        '^--pedestrian-vat-debug$' {
            if ($nextIsValue) { $PedestrianVatDebugMode = $next; $i++ }
            else { $PedestrianVatDebugMode = "rest" }
        }

        '^--debug$' {
            if ($nextIsValue) {
                $cat = $next
                $i++
                switch ($cat) {
                    "traffic" { $DebugTraffic = $true }
                    { $_ -in @("site-grading", "building-site-grading") } {
                        $DebugOn = $true; $DebugBuildings = $true
                        $BuildingSiteVisualDebug = $true; $DebugCategory = "road"
                    }
                    { $_ -in @("buildings", "building-sites") } {
                        $DebugOn = $true; $DebugBuildings = $true; $DebugCategory = "buildings"
                    }
                    { $_ -in @("building-sites-visual", "building-visual", "buildings-visual") } {
                        $BuildingSiteVisualDebug = $true
                        $mode = if ($i + 1 -lt $ArgList.Count) { $ArgList[$i + 1] } else { $null }
                        if (($null -ne $mode) -and (-not $mode.StartsWith("--"))) {
                            $BuildingSiteVisualDebugMode = $mode; $i++
                        }
                    }
                    { $_ -in @("visuals", "visual") } {
                        $VisualDebug = $true
                        $mode = if ($i + 1 -lt $ArgList.Count) { $ArgList[$i + 1] } else { $null }
                        if (($null -ne $mode) -and (-not $mode.StartsWith("--"))) {
                            $VisualDebugMode = $mode; $i++
                        }
                    }
                    "terrain-visual" {
                        $TerrainDebug = $true
                        $mode = if ($i + 1 -lt $ArgList.Count) { $ArgList[$i + 1] } else { $null }
                        if (($null -ne $mode) -and (-not $mode.StartsWith("--"))) {
                            $TerrainVisualMode = $mode; $i++
                        }
                    }
                    { $_ -in @("terrain", "terrain-verbose", "terrain-full", "terrain-lod1", "terrain-full-lod1") } {
                        Set-TerrainPreset $cat
                    }
                    "perf"  { $DebugPerf = $true }
                    "road"  { $DebugOn = $true; $DebugCategory = "road"; $DebugRoadGeometryDump = $true; $DebugSurface = $true }
                    default { $DebugOn = $true; $DebugCategory = $cat }
                }
            } else {
                $DebugOn = $true
            }
        }

        # Anything unrecognised goes through to the game, matching run.sh.
        default { $GodotArgs.Add($arg) }
    }
    $i++
}

# --------------------------------------------------------- environment setup
if ($DebugOn)                 { $env:METRUM_DEBUG = "1" }
if ($DebugCategory -ne "")    { $env:METRUM_DEBUG_FILTER = $DebugCategory }
if ($DebugTraffic)            { $env:METRUM_DEBUG_TRAFFIC = "1" }
if ($DebugSim)                { $env:METRUM_DEBUG_SIM = "1" }
if ($DebugPerf)               { $env:METRUM_DEBUG_PERF = "1" }
if ($DebugBuildings)          { $env:METRUM_DEBUG_BUILDINGS = "1" }
if ($DebugSurface)            { $env:METRUM_DEBUG_SURFACE = "1" }
if ($DebugRoadGeometryDump)   { $env:METRUM_DEBUG_ROAD_GEOMETRY_DUMP = "1" }
if ($VisualDebug)             { $env:METRUM_DEBUG_TERRAIN_GRASS = $VisualDebugMode }
if ($TerrainDebug)            { $env:METRUM_DEBUG_TERRAIN = "1" }
if ($TerrainVerbose)          { $env:METRUM_DEBUG_TERRAIN_VERBOSE = "1" }
if ($TerrainForceFullWorld)   { $env:METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD = "1" }
if ($TerrainForceLod -ne "")  { $env:METRUM_DEBUG_TERRAIN_FORCE_LOD = $TerrainForceLod }
if ($TerrainVisualMode -ne ""){ $env:METRUM_DEBUG_TERRAIN_VISUAL = $TerrainVisualMode }
if ($BuildingSiteVisualDebug) { $env:METRUM_DEBUG_BUILDING_SITES_VISUAL = $BuildingSiteVisualDebugMode }
if ($PedestrianVatDebugMode -ne "") { $env:METRUM_DEBUG_PEDESTRIAN_VAT = $PedestrianVatDebugMode }

if ($Release -and -not $env:METRUM_CRASH_DIAGNOSTICS) {
    $env:METRUM_CRASH_DIAGNOSTICS = "1"
}
if ($env:METRUM_CRASH_DIAGNOSTICS -eq "1" -and -not $env:METRUM_CRASH_LOG_DIR) {
    $env:METRUM_CRASH_LOG_DIR = Join-Path $ProjectRoot "logs"
    if (-not (Test-Path $env:METRUM_CRASH_LOG_DIR)) {
        New-Item -ItemType Directory -Force $env:METRUM_CRASH_LOG_DIR | Out-Null
    }
}

# ------------------------------------------------------------- locate Godot
function Resolve-Godot {
    if ($Godot -ne "") {
        if (Test-Path $Godot) { return $Godot }
        throw "Godot not found at -Godot path: $Godot"
    }
    if ($env:GODOT) {
        if (Test-Path $env:GODOT) { return $env:GODOT }
        throw "Godot not found at `$env:GODOT: $env:GODOT"
    }

    foreach ($name in @("godot", "Godot_v4.7.1-stable_mono_win64_console", "Godot_v4.7.1-stable_mono_win64")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd.Source }
    }

    # Console variants print to stdout on Windows, so prefer them.
    $candidates = @(
        "C:\Program Files\Godot Sharp v4.7.1\Godot_v4.7.1-stable_mono_win64_console.exe",
        "$env:LOCALAPPDATA\Programs\Godot\Godot_v4.7.1-stable_win64_console.exe"
    )
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }

    throw "Godot 4.7 not found. Pass -Godot <path> or set `$env:GODOT."
}

$GodotExe = Resolve-Godot

# ----------------------------------------------------------------- building
Write-Host "Building Rust library..."
Push-Location $RustDir
try {
    # cargo writes progress to stderr; with ErrorActionPreference=Stop that
    # surfaces as a NativeCommandError even on success, so relax it for the
    # build and judge the result by the exit code, which actually reports it.
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    if ($Release) {
        & cmd /c "cargo build --release 2>&1"
        $buildCode = $LASTEXITCODE
        $Lib = Join-Path $RustDir (Join-Path "target" (Join-Path "release" "metrum_rise.dll"))
    } else {
        & cmd /c "cargo build 2>&1"
        $buildCode = $LASTEXITCODE
        $Lib = Join-Path $RustDir (Join-Path "target" (Join-Path "debug" "metrum_rise.dll"))
    }
    $ErrorActionPreference = $prevEap
    if ($buildCode -ne 0) { Write-Host "Rust build failed!"; exit 1 }
} finally {
    Pop-Location
}

if (-not (Test-Path $Lib)) { Write-Host "Built library missing: $Lib"; exit 1 }

Write-Host "Deploying library..."
$BinDir = Join-Path $GodotDir "bin"
if (-not (Test-Path $BinDir)) { New-Item -ItemType Directory -Force $BinDir | Out-Null }
Copy-Item $Lib (Join-Path $BinDir "metrum_rise.dll") -Force

Write-Host "Registering GDExtension..."
$DotGodot = Join-Path $GodotDir ".godot"
if (-not (Test-Path $DotGodot)) { New-Item -ItemType Directory -Force $DotGodot | Out-Null }
Set-Content -Path (Join-Path $DotGodot "extension_list.cfg") `
            -Value "res://bin/metrum_rise.gdextension" -Encoding utf8 -NoNewline

# ------------------------------------------------------------------ launch
Write-Host "Launching Metrum Rise..."
Push-Location $GodotDir
try {
    $all = @()
    $all += $GodotEngineArgs
    if ($GodotArgs.Count -gt 0) { $all += "--"; $all += $GodotArgs }
    & $GodotExe @all
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $code

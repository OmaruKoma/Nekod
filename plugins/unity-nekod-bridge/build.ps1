param(
    [ValidateSet("IL2CPP", "Mono")]
    [string]$Target = "IL2CPP",
    [Parameter(Mandatory = $true)]
    [string]$UnityGameDir
)

$ErrorActionPreference = "Stop"
$cfg = if ($Target -eq "IL2CPP") { "Release_IL2CPP" } else { "Release_Mono" }
Write-Host "Building NekodUnityBridge ($cfg) against '$UnityGameDir'..."
dotnet build "$PSScriptRoot\NekodUnityBridge.csproj" -c $cfg "-p:UnityGameDir=$UnityGameDir"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "Build succeeded. Copy the DLL from plugins\unity-nekod-bridge\dist\ into <Game>\BepInEx\plugins\."

<#
.SYNOPSIS
  Baut den Windows-Installer (NSIS) des Lykos-Desktop-Clients samt signierten Update-Artefakten
  und legt alles unter sprechenden Namen in dist-desktop/ ab.

.DESCRIPTION
  Version kommt aus client/src-tauri/tauri.conf.json. Ergebnis z. B.:
    dist-desktop/Lykos-0.3.0-windows-x64-setup.exe          Installer (zugleich Update-Paket)
    dist-desktop/Lykos-0.3.0-windows-x64-setup.exe.sha256   Prüfsumme (Download-Seite)
    dist-desktop/Lykos-0.3.0-windows-x64-setup.exe.sig      Updater-Signatur (Update-Manifest)
    dist-desktop/latest.json                                 Statisches Tauri-Update-Manifest
  dist-desktop/ ist per .gitignore ausgeschlossen. Der Upload als Release (GitHub ZeasySoftware/Lykos) passiert separat:
  .exe, .sha256 und .sig als Assets anhängen. Der Lykos-Server baut daraus GET /api/desktop/latest
  und /api/desktop/latest.json (LYKOS_DESKTOP_RELEASES_URL); latest.json liegt nur zur Kontrolle bzw.
  für statisches Hosting bei.

  Signiert wird mit dem Tauri-Updater-Schlüssel (minisign), standardmäßig
  %USERPROFILE%\.tauri\lykos-updater.key; das Passwort steht in derselben Datei mit Endung
  .password. Der Schlüssel darf nie ins Repo – ohne ihn (bzw. bei Verlust) können installierte
  Apps keine Updates mehr annehmen.

.PARAMETER SkipInstall
  `npm ci` überspringen (node_modules ist schon aktuell).

.PARAMETER SigningKey
  Pfad zum privaten Updater-Schlüssel.

.PARAMETER Notes
  Versionshinweise für latest.json (der Server nimmt stattdessen den Text des Releases).

.PARAMETER ReleaseBaseUrl
  Download-Basis der Release-Assets; die Installer-URL ist <Basis>/v<Version>/<Datei>.

.EXAMPLE
  pwsh scripts/release-desktop.ps1
#>
param(
  [switch]$SkipInstall,
  [string]$SigningKey = (Join-Path $env:USERPROFILE '.tauri\lykos-updater.key'),
  [string]$Notes = '',
  [string]$ReleaseBaseUrl = 'https://github.com/ZeasySoftware/Lykos/releases/download'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$client = Join-Path $root 'client'
$confPath = Join-Path $client 'src-tauri\tauri.conf.json'
$conf = Get-Content $confPath -Raw -Encoding utf8 | ConvertFrom-Json
$version = $conf.version
$product = $conf.productName
if (-not $version) { throw "Keine Version in $confPath" }

if (-not (Test-Path $SigningKey)) {
  throw "Updater-Schlüssel $SigningKey fehlt. Neu erzeugen nur, wenn er wirklich verloren ist (dann nehmen installierte Apps keine Updates mehr an): npx tauri signer generate -w `"$SigningKey`" -p <Passwort>"
}
$passwordFile = "$SigningKey.password"
$password = if (Test-Path $passwordFile) { (Get-Content $passwordFile -Raw).Trim() } else { '' }
if (-not $password) { throw "Passwort-Datei $passwordFile fehlt oder ist leer." }

Write-Host "==> $product ${version}: Desktop-Installer und Update-Artefakte bauen" -ForegroundColor Cyan

Push-Location $client
try {
  if (-not $SkipInstall) {
    npm ci --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'npm ci fehlgeschlagen' }
  }
  # createUpdaterArtifacts (tauri.conf.json) signiert den Installer; der Schlüssel kommt nur über
  # Umgebungsvariablen dieses Prozesses in den Build.
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $SigningKey -Raw
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $password
  # beforeBuildCommand (npm run build) baut den Web-Client mit.
  npm run tauri -- build --bundles nsis
  if ($LASTEXITCODE -ne 0) { throw 'tauri build fehlgeschlagen' }
} finally {
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  Pop-Location
}

$bundleDir = Join-Path $client 'src-tauri\target\release\bundle\nsis'
$installer = Get-ChildItem $bundleDir -Filter "*_$version*-setup.exe" |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if (-not $installer) { throw "Installer für Version $version nicht in $bundleDir gefunden" }
$signature = "$($installer.FullName).sig"
if (-not (Test-Path $signature)) { throw "Updater-Signatur $signature fehlt (createUpdaterArtifacts?)" }

$arch = if ($installer.Name -match '_(x64|x86|arm64)-setup\.exe$') { $Matches[1] } else { 'x64' }
$platform = @{ x64 = 'windows-x86_64'; x86 = 'windows-i686'; arm64 = 'windows-aarch64' }[$arch]
$outDir = Join-Path $root 'dist-desktop'
New-Item -ItemType Directory -Force $outDir | Out-Null
$target = Join-Path $outDir "$product-$version-windows-$arch-setup.exe"
Copy-Item $installer.FullName $target -Force
# Die Signatur gilt für den Dateiinhalt, nicht den Namen – Umbenennen ist unkritisch.
Copy-Item $signature "$target.sig" -Force

$file = Get-Item $target
$hash = (Get-FileHash $target -Algorithm SHA256).Hash.ToLower()
"$hash  $($file.Name)" | Out-File -Encoding ascii "$target.sha256"

# Statisches Tauri-Update-Manifest (UTF-8 ohne BOM).
$manifest = [ordered]@{
  version   = $version
  notes     = $Notes
  pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
  platforms = [ordered]@{
    $platform = [ordered]@{
      signature = (Get-Content $signature -Raw).Trim()
      url       = "$ReleaseBaseUrl/v$version/$($file.Name)"
    }
  }
}
$manifestPath = Join-Path $outDir 'latest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))

Write-Host ''
Write-Host "Installer: $($file.FullName)" -ForegroundColor Green
Write-Host ("Größe:     {0:N1} MB ({1} Bytes)" -f ($file.Length / 1MB), $file.Length)
Write-Host "SHA-256:   $hash"
Write-Host "Signatur:  $target.sig"
Write-Host "Manifest:  $manifestPath"
Write-Host ''
Write-Host "Release auf GitHub: gh release create v$version --repo ZeasySoftware/Lykos --prerelease <.exe> <.exe.sha256> <.exe.sig>" -ForegroundColor Cyan

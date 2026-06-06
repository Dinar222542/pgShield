# Run this as Administrator to enable localhost access to pgShield
param(
    [string]$WslIp = "",
    [int]$Port = 8080
)

if (-NOT ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole] "Administrator")) {
    Write-Host "ERROR: Must run as Administrator" -ForegroundColor Red
    exit 1
}

if (-not $WslIp) {
    $ipMatch = wsl -- ip addr show eth0 2>&1 | Select-String -Pattern "inet (\d+\.\d+\.\d+\.\d+)"
    $WslIp = [regex]::Match($ipMatch, "(\d+\.\d+\.\d+\.\d+)").Groups[1].Value
    if (-not $WslIp) {
        Write-Host "ERROR: Could not detect WSL2 IP" -ForegroundColor Red
        exit 1
    }
}

# Remove old rule if exists
netsh interface portproxy delete v4tov4 listenport=$Port listenaddress=0.0.0.0 2>$null

# Add new rule
netsh interface portproxy add v4tov4 listenport=$Port listenaddress=0.0.0.0 connectport=$Port connectaddress=$WslIp
if ($LASTEXITCODE -eq 0) {
    Write-Host "OK: Port $Port forwarded from Windows -> WSL2 ($WslIp)" -ForegroundColor Green
    Write-Host "Access pgShield at: http://localhost:$Port"
} else {
    Write-Host "ERROR: Failed to set up portproxy" -ForegroundColor Red
    exit 1
}

# Run as Administrator — adds portproxy + firewall rule
$wslIp = (wsl -- ip addr show eth0 2>$null | Select-String -Pattern 'inet\s+(\d+\.\d+\.\d+\.\d+)').Matches.Groups[1].Value
if (-not $wslIp) { $wslIp = "172.18.243.16" }

netsh interface portproxy delete v4tov4 listenport=8080 2>$null
netsh interface portproxy add v4tov4 listenport=8080 listenaddress=0.0.0.0 connectport=8080 connectaddress=$wslIp

netsh advfirewall firewall add rule name="pgShield Web UI" dir=in action=allow protocol=TCP localport=8080 2>$null

Write-Output "Port 8080 forwarded to WSL ($wslIp), firewall opened."
Write-Output "Your LAN IP:"
(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -ne 'Loopback' -and $_.PrefixOrigin -ne 'Link' }).IPAddress

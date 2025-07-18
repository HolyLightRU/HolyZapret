@echo off
chcp 65001 > nul

cd /d "%~dp0.."  

set "BIN=%CD%\bin\"
set "LISTS=%CD%\lists\"

powershell -Command "Write-Host 'Экспериментальные настройки' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'Перебирайте все additional по очереди' -ForegroundColor Magenta"

"%BIN%HolyZapret.exe" --wf-tcp=80,443 --wf-tcp=80,443 --wf-udp=443,50000-50100 ^
--filter-udp=443 --hostlist="list-general.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-udp=50000-50100 --ipset="ipset-discord.txt" --dpi-desync=fake --dpi-desync-any-protocol --dpi-desync-cutoff=d3 --dpi-desync-repeats=6 --new ^
--filter-tcp=80 --hostlist="list-general.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=443 --hostlist="list-general.txt" --dpi-desync=split2 --dpi-desync-split-seqovl=652 --dpi-desync-split-pos=2 --dpi-desync-split-seqovl-pattern="%BIN%tls_clienthello_www_google_com.bin"

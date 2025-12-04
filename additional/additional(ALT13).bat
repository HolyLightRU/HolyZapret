@echo off
chcp 65001 > nul

cd /d "%~dp0.."  

set "BIN=%CD%\bin\"
set "LISTS=%CD%\exp-list\"

powershell -Command "Write-Host 'Экспериментальные настройки' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'Перебирайте все additional по очереди' -ForegroundColor Magenta"
	
"%BIN%HolyZapret.exe" --wf-tcp=80,443,2053,2083,2087,2096,8443 --wf-udp=443,1400,19294-19344,50000-50100 ^
--comment Telegram Calls --filter-udp=1400 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-repeats=6 --new ^
--filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-repeats=6 --new ^
--filter-tcp=80 --dpi-desync=fake,multisplit --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --dpi-desync=multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-split-seqovl-pattern="%BIN%4pda.bin" --new ^
--filter-tcp=443 --dpi-desync=multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-split-seqovl-pattern="%BIN%4pda.bin" --new ^
--filter-udp=443 --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin"
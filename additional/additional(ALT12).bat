@echo off
chcp 65001 > nul

cd /d "%~dp0.."  

set "BIN=%CD%\bin\"
set "LISTS=%CD%\exp-list\"

powershell -Command "Write-Host 'Экспериментальные настройки' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'Перебирайте все additional по очереди' -ForegroundColor Magenta"
	
"%BIN%HolyZapret.exe" --wf-tcp=80,443,2053,2083,2087,2096,8443 --wf-udp=443,19294-19344,50000-50100 ^
--filter-tcp=80 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=2 --new ^
--filter-tcp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-fake-tls-mod=none --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=2 --new ^
--filter-udp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-tcp=80 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=10000000 --new ^
--filter-tcp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-fake-tls-mod=none --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=10000000 --new ^
--filter-udp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --hostlist-domains=discord.media --dpi-desync=fake --dpi-desync-fake-tls-mod=none --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=2 --new ^
--filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-repeats=6 --new ^
--filter-tcp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-fake-tls-mod=none --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=10000000
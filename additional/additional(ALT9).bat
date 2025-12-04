@echo off
chcp 65001 > nul

cd /d "%~dp0.."  

set "BIN=%CD%\bin\"
set "LISTS=%CD%\lists\"

powershell -Command "Write-Host 'Экспериментальные настройки' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'Перебирайте все additional по очереди' -ForegroundColor Magenta"
	
"%BIN%HolyZapret.exe" --wf-tcp=80,443,5222,8883,853 --wf-udp=443,50000-50100,5222,8883,853 ^
--filter-tcp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,multidisorder --dpi-desync-repeats=8 --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com --dpi-desync-fooling=md5sig,hopbyhop2,badseq --new ^
--filter-tcp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake,split2 --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=md5sig,hopbyhop2,badseq --dpi-desync-ttl=3 --new ^
--filter-tcp=80,5222,8883,853 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=3 --dpi-desync-fooling=md5sig,hopbyhop2,badseq --new ^
--filter-tcp=80,5222,8883,853 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig,hopbyhop2,badseq --new ^
--filter-udp=443,5222,8883,853 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=8 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --dpi-desync-fooling=md5sig,hopbyhop2,badseq --new ^
--filter-udp=443,5222,8883,853 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-repeats=8 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --dpi-desync-any-protocol=1 --dpi-desync-cutoff=n2 --new ^
--filter-udp=50000-50100,5222,8883,853 --filter-l7=discord,stun,wireguard --dpi-desync=fake --dpi-desync-repeats=6 --new ^
--filter-udp=50000-50100 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-autottl=2 --dpi-desync-repeats=10 --dpi-desync-any-protocol=1 --dpi-desync-fake-unknown-udp="%BIN%quic_initial_www_google_com.bin" --dpi-desync-cutoff=n2
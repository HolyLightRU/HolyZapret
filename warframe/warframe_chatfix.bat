@echo off
chcp 65001 > nul

cd /d "%~dp0.."  

set "BIN=%CD%\bin\"
set "LISTS=%CD%\lists\"

powershell -Command "Write-Host 'Экспериментальные настройки. Фикс чата для Warframe' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'По всем вопросам в ТГ: @HolyLightRU' -ForegroundColor Magenta"
powershell -Command "Write-Host 'Мои "социальные" сети:' -ForegroundColor White"
powershell -Command "Write-Host 'https://t.me/notholylab' -ForegroundColor Cyan"
powershell -Command "Write-Host 'https://github.com/HolyLightRU' -ForegroundColor White"
powershell -Command "Write-Host 'Так-же в папках additional и flowseal_strat' -ForegroundColor Magenta"
powershell -Command "Write-Host 'Лежат дополнительные стратегии' -ForegroundColor DarkMagenta"

"%BIN%HolyZapret.exe" --wf-tcp=80,443,6695-6705 --wf-udp=443,50000-50100 ^
--filter-udp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-udp=50000-50100 --filter-l7=discord,stun --dpi-desync=fake --new ^
--filter-tcp=80 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=split --dpi-desync-split-pos=1 --dpi-desync-autottl --dpi-desync-fooling=badseq --dpi-desync-repeats=8 --new ^
--filter-udp=443 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-tcp=80 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=443 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=split --dpi-desync-split-pos=1 --dpi-desync-autottl --dpi-desync-fooling=badseq --dpi-desync-repeats=8 --new ^
--filter-tcp=6695-6705 --ipset-ip=172.232.25.131 --dpi-desync=fakedsplit --dpi-desync-split-pos=1 --dpi-desync-ttl=0 --dpi-desync-repeats=16 --dpi-desync-fooling=md5sig,badsum --dpi-desync-fake-tls-mod=rnd,rndsni,padencap

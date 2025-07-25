@echo off
chcp 65001 > nul
:: 65001 - UTF-8

cd /d "%~dp0"

set "BIN=%~dp0bin\"
set "LISTS=%~dp0lists\"
powershell -NoProfile -Command ^"Start-Process -FilePath 'holy_host_system.bat' -Verb runAs"

powershell -Command "Write-Host 'Самые быстрые фиксы беспредела РКН' -ForegroundColor DarkMagenta"
powershell -Command "Write-Host 'Расчитано и проверяется в Сибирском регионе' -ForegroundColor Magenta"
powershell -Command "Write-Host 'Мои "социальные" сети:' -ForegroundColor White"
powershell -Command "Write-Host 'https://t.me/notholylab' -ForegroundColor Cyan"
powershell -Command "Write-Host 'https://github.com/HolyLightRU' -ForegroundColor White"
powershell -Command "Write-Host 'Так-же в папках additional и flowseal_strat' -ForegroundColor Magenta"
powershell -Command "Write-Host 'Лежат дополнительные стратегии' -ForegroundColor DarkMagenta"

"%BIN%HolyZapret.exe" --wf-tcp=80,443,2050-2099 --wf-udp=443,50000-50099,0-65535 ^
--comment Discord(Calls) --filter-udp=50000-50099 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-fake-discord=0x00 --dpi-desync-fake-stun=0x00 --dpi-desync-repeats=6 --new ^

--comment Discord (Calls Extra) --filter-tcp=443,2050-2099 --hostlist-domains=discord.media --dpi-desync=rst,multidisorder --dpi-desync-fooling=badseq --new ^

--comment Discord --filter-tcp=443 --hostlist-domains=dis.gd,discord-attachments-uploads-prd.storage.googleapis.com,discord.app,discord.co,discord.com,discord.design,discord.dev,discord.gift,discord.gifts,discord.gg,gateway.discord.gg,discord.media,discord.new,discord.store,discord.status,discord-activities.com,discordactivities.com,discordapp.com,cdn.discordapp.com,discordapp.net,media.discordapp.net,images-ext-1.discordapp.net,stable.dl2.discordapp.net,discordcdn.com,discordmerch.com,discordpartygames.com,discordsays.com,discordsez.com --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=0 --new ^

--comment YouTube QUIC/QUIC --filter-udp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^

--comment YouTube Streaming/HTTP --filter-tcp=80 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,multisplit --dpi-desync-fooling=badseq --new ^

--comment YouTube --filter-tcp=443 --hostlist-domains=yt3.ggpht.com,yt4.ggpht.com,yt3.googleusercontent.com,googlevideo.com,jnn-pa.googleapis.com,wide-youtube.l.google.com,youtube-nocookie.com,youtube-ui.l.google.com,youtube.com,youtubeembeddedplayer.googleapis.com,youtubekids.com,youtubei.googleapis.com,youtu.be,yt-video-upload.l.google.com,ytimg.com,ytimg.l.google.com --dpi-desync=multisplit --dpi-desync-split-seqovl=675 --dpi-desync-split-pos=1,midsld --dpi-desync-split-seqovl-pattern="%BIN%tls_clienthello_www_google_com.bin" --new ^

--comment list-general+extra --filter-tcp=443 --hostlist-exclude-domains=dis.gd,discord-attachments-uploads-prd.storage.googleapis.com,discord.app,discord.co,discord.com,discord.design,discord.dev,discord.gift,discord.gifts,discord.gg,gateway.discord.gg,discord.media,discord.new,discord.store,discord.status,discord-activities.com,discordactivities.com,discordapp.com,cdn.discordapp.com,discordapp.net,media.discordapp.net,images-ext-1.discordapp.net,stable.dl2.discordapp.net,discordcdn.com,discordmerch.com,discordpartygames.com,discordsays.com,discordsez.com,yt3.ggpht.com,yt4.ggpht.com,yt3.googleusercontent.com,googlevideo.com,jnn-pa.googleapis.com,wide-youtube.l.google.com,youtube-nocookie.com,youtube-ui.l.google.com,youtube.com,youtubeembeddedplayer.googleapis.com,youtubekids.com,youtubei.googleapis.com,youtu.be,yt-video-upload.l.google.com,ytimg.com,ytimg.l.google.com --hostlist="%LISTS%list-general.txt" --hostlist-domains=adguard.com,adguard-vpn.com,totallyacdn.com,whiskergalaxy.com,windscribe.com,windscribe.net,cloudflareclient.com,soundcloud.com,sndcdn.com,soundcloud.cloud,nexusmods.com,nexus-cdn.com,supporter-files.nexus-cdn.com,prostovpn.org,hitmotop.com --dpi-desync=fake,multisplit --dpi-desync-split-pos=1 --dpi-desync-fooling=badseq --new ^

--filter-tcp=443 --hostlist-domains=adguard.com,adguard-vpn.com,totallyacdn.com,whiskergalaxy.com,windscribe.com,windscribe.net,cloudflareclient.com,soundcloud.com,sndcdn.com,soundcloud.cloud --dpi-desync=split2 --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-repeats=2 --dpi-desync-fooling=badseq,hopbyhop2 --dpi-desync-split-seqovl-pattern="%BIN%tls_clienthello_www_google_com.bin" --new^

--comment Cloudflare WARP Gateway(1.1.1.1, 1.0.0.1) --filter-tcp=443 --ipset-ip=162.159.36.1,162.159.46.1,2606:4700:4700::1111,2606:4700:4700::1001 --filter-l7=tls --dpi-desync=fake --dpi-desync-fake-tls=0x00 --dpi-desync-start=n2 --dpi-desync-cutoff=n3 --dpi-desync-fooling=badseq --dpi-desync-badseq-increment=0 --new ^

--comment WireGuard handshake --filter-udp=0-65535 --filter-l7=wireguard --dpi-desync=fake --dpi-desync-repeats=4 --dpi-desync-fake-wireguard=0x00 --dpi-desync-cutoff=n2 --new ^

--comment IP set --filter-tcp=80 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake,multisplit --dpi-desync-fooling=badseq --new ^

--comment IP set --filter-tcp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fooling=badseq --new ^

--comment IP set --filter-udp=443 --ipset="%LISTS%ipset-all.txt" --dpi-desync=fake --dpi-desync-repeats=6 --new ^

--filter-udp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-udp=50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-repeats=6 --new ^
--filter-tcp=80 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake,split2 --dpi-desync-repeats=6 --dpi-desync-fooling=md5sig --dpi-desync-fake-tls="%BIN%tls_clienthello_www_google_com.bin" --new ^
--filter-udp=443 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-tcp=80 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=fake,split2 --dpi-desync-autottl=2 --dpi-desync-fooling=md5sig --new ^
--filter-tcp=443 --ipset="%LISTS%ipset-cloudflare.txt" --dpi-desync=fake,split2 --dpi-desync-repeats=6 --dpi-desync-fooling=md5sig --dpi-desync-fake-tls="%BIN%tls_clienthello_www_google_com.bin"
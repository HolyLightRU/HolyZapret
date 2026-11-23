pub fn pac(addr: &str, on: bool) -> String {
    if !on {
        return "function FindProxyForURL(url, host) {\n  return \"DIRECT\";\n}\n".into();
    }
    format!(
        "function FindProxyForURL(url, host) {{\n\
         \x20 if (isPlainHostName(host) || shExpMatch(host, \"*.local\")) return \"DIRECT\";\n\
         \x20 var ip = dnsResolve(host);\n\
         \x20 if (ip && (isInNet(ip, \"10.0.0.0\", \"255.0.0.0\") || isInNet(ip, \"172.16.0.0\", \"255.240.0.0\") ||\n\
         \x20     isInNet(ip, \"192.168.0.0\", \"255.255.0.0\") || isInNet(ip, \"127.0.0.0\", \"255.0.0.0\"))) return \"DIRECT\";\n\
         \x20 return \"PROXY {addr}; DIRECT\";\n\
         }}\n"
    )
}

pub fn v2ray_link(addr: &str) -> String {
    format!("socks://{addr}#HolyZapret")
}

pub fn html(addr: &str, on: bool) -> String {
    let (ip, port) = addr.rsplit_once(':').unwrap_or((addr, ""));
    let link = v2ray_link(addr);
    let state = if on {
        ""
    } else {
        r#"<div class="card warn"><b>Раздача сейчас выключена.</b> Если на устройстве выбран прокси вручную, уберите его, иначе интернет на этой сети не будет работать.</div>"#
    };
    format!(
        r#"<!doctype html><html lang="ru"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>HolyZapret: обход на телефоне и ТВ</title>
<style>
body{{margin:0;font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;background:#141018;color:#eee;line-height:1.5}}
main{{max-width:560px;margin:0 auto;padding:20px 16px 40px}}
h1{{font-size:22px;margin:4px 0 6px;background:linear-gradient(90deg,#f28fc9,#69cdf5);-webkit-background-clip:text;background-clip:text;color:transparent}}
.card{{background:#201b27;border:1px solid #2e2836;border-radius:14px;padding:14px 16px;margin:14px 0}}
.addr{{font-size:22px;font-weight:600;letter-spacing:.3px}}
code{{background:#2b2433;border-radius:6px;padding:2px 6px;font-size:15px;overflow-wrap:anywhere}}
ol{{padding-left:20px;margin:6px 0}}li{{margin:6px 0}}
.muted{{color:#a59fae;font-size:14px}}
.warn{{background:#3a2a12;border-color:#7a5a20;color:#f6d79a}}
.warn b{{color:#ffe2a8}}
button,.btn{{background:linear-gradient(90deg,#f28fc9,#69cdf5);border:0;border-radius:10px;padding:10px 14px;font-size:15px;color:#24122a;font-weight:600;text-decoration:none;display:inline-block}}
.btn{{margin:10px 0 4px}}
.tabs{{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0 0}}
.tabs button{{background:#2b2433;color:#ddd;font-weight:500;padding:8px 12px}}
.tabs button.on{{background:linear-gradient(90deg,#f28fc9,#69cdf5);color:#24122a;font-weight:600}}
section{{display:none}}section.on{{display:block}}
#v2{{display:block;margin:6px 0;font-size:13px}}
</style></head><body><main>
<h1>Обход HolyZapret на других устройствах</h1>
<div class="muted">Устройство пойдёт в интернет через компьютер с HolyZapret. Выберите, что настраиваете:</div>
<div class="tabs"><button data-d="android">Android</button><button data-d="iphone">iPhone</button><button data-d="tv">Телевизор</button><button data-d="ps">PlayStation</button></div>
{state}
<div class="card"><div class="muted">Адрес прокси</div><div class="addr">{ip} : {port}</div></div>

<section data-d="android">
<div class="card"><b>Способ 1. Приложение v2rayNG (рекомендуем)</b>
<div class="muted">Через компьютер пойдёт всё: браузер, приложения и звонки Discord. Выключили v2rayNG — интернет снова идёт напрямую, даже если компьютер выключен.</div>
<ol><li>Установите v2rayNG из Google Play или с GitHub.</li>
<li>Скопируйте ссылку:<br><code id="v2">{link}</code> <button onclick="cp('v2',this)">Копировать</button></li>
<li>В v2rayNG нажмите плюс в правом верхнем углу → <b>Импорт из буфера обмена</b>. Появится сервер HolyZapret.</li>
<li>Выберите его и нажмите кнопку запуска внизу. Android спросит разрешение на VPN, разрешите.</li></ol>
<div class="muted">Вручную: плюс → «Вручную: SOCKS», адрес <code>{ip}</code>, порт <code>{port}</code>, логин и пароль пустые. Подойдёт и любой другой клиент с SOCKS5 и UDP, например NekoBox.</div></div>
<div class="card"><b>Способ 2. Прокси в настройках Wi-Fi, без приложений</b>
<a class="btn" href="intent:#Intent;action=android.settings.WIFI_SETTINGS;end">Открыть настройки Wi-Fi</a>
<div class="muted">Если кнопка не сработала, откройте настройки вручную.</div>
<ol><li>Настройки → Wi-Fi → нажмите на вашу сеть → Изменить (или шестерёнка).</li>
<li>Дополнительно → Прокси → <b>Автоматическая настройка</b> (Auto-config, PAC).</li>
<li>В поле адреса PAC впишите:<br><code id="pac">http://{addr}/proxy.pac</code> <button onclick="cp('pac',this)">Копировать</button></li>
<li>Сохраните. Если в HolyZapret выключить раздачу, телефон пойдёт в интернет напрямую.</li></ol>
<div class="muted">Или Прокси → <b>Вручную</b>: имя хоста <code id="ip">{ip}</code> <button onclick="cp('ip',this)">Копировать</button>, порт <code id="port">{port}</code> <button onclick="cp('port',this)">Копировать</button>.</div>
<div class="muted">Так работают браузер и большинство приложений, но не звонки.</div></div>
<div class="card warn"><b>Важно для способа 2.</b> Пока прокси включён, Wi-Fi этой сети работает только когда компьютер включён и в HolyZapret запущена раздача. Android не подскажет, что дело в прокси, он просто покажет «Нет интернета».<br><br><b>Как вернуть интернет:</b> Настройки → Wi-Fi → нажмите на вашу сеть → Изменить → Дополнительно → Прокси → <b>Нет</b>. Мобильный интернет прокси не затрагивает.</div>
</section>

<section data-d="iphone">
<div class="card warn"><b>Важно.</b> Пока прокси включён, Wi-Fi этой сети работает только с включённым компьютером и открытым HolyZapret.<br><br><b>Как вернуть интернет:</b> Настройки → Wi-Fi → ⓘ рядом с сетью → Настройка прокси → <b>Выкл.</b></div>
<div class="card"><b>Настройка</b>
<ol><li>Настройки → Wi-Fi → нажмите ⓘ рядом с вашей сетью.</li>
<li>В самом низу: Настройка прокси → <b>Авто</b>.</li>
<li>В поле URL впишите:<br><code id="ipac">http://{addr}/proxy.pac</code> <button onclick="cp('ipac',this)">Копировать</button></li>
<li>Нажмите «Сохранить». Если в HolyZapret выключить раздачу, iPhone пойдёт в интернет напрямую.</li></ol>
<div class="muted">Если «Авто» не заработало, выберите «Вручную»: сервер <code>{ip}</code>, порт <code>{port}</code>, аутентификация выключена.</div></div>
</section>

<section data-d="tv">
<div class="card warn"><b>Подумайте, прежде чем включать.</b> Телевизор с этим прокси выходит в интернет только когда компьютер включён и в HolyZapret запущена раздача. Если компьютер выключен, на телевизоре не будет работать ничего, включая обновления и другие приложения.<br><br><b>Как вернуть интернет:</b> там же, где включали, выберите Прокси → <b>Нет</b>.</div>
<div class="card"><b>Android TV и Google TV (Xiaomi Mi TV, Sony, TCL, Philips, приставки Mi Box)</b>
<ol><li>Настройки → Сеть и Интернет.</li>
<li>Выберите вашу Wi-Fi сеть. На некоторых моделях: удерживайте её или откройте «Дополнительно».</li>
<li>Настройки прокси-сервера → <b>Вручную</b>.</li>
<li>Имя хоста прокси: <code id="tip">{ip}</code></li>
<li>Порт: <code id="tport">{port}</code></li>
<li>Строку «Обходить прокси для» оставьте пустой и сохраните.</li></ol>
<div class="muted">Если телевизор подключён кабелем, прокси ищите в настройках Ethernet. Набирать адрес пультом долго, удобнее подключить к телевизору телефон как пульт через приложение Google TV.</div></div>
<div class="card muted">Проверено не на всех моделях. Если YouTube на телевизоре не заработал, напишите нам модель телевизора.</div>
</section>

<section data-d="ps">
<div class="card warn"><b>Важно.</b> С прокси приставка выходит в интернет только при включённом компьютере с открытым HolyZapret. Онлайн в играх может работать хуже: игры обычно ходят мимо прокси. Включайте прокси для YouTube и магазина, а для игр лучше выключить.</div>
<div class="card"><b>PlayStation 5</b>
<ol><li>Настройки → Сеть → Настройки → Настроить соединение с Интернетом.</li>
<li>Наведите на вашу сеть, нажмите кнопку Options → Расширенные настройки.</li>
<li>Прокси-сервер → <b>Использовать</b>.</li>
<li>Адрес: <code id="pip">{ip}</code>, порт: <code id="pport">{port}</code>. Нажмите OK.</li></ol></div>
<div class="card"><b>PlayStation 4</b>
<ol><li>Настройки → Сеть → Установить соединение с Интернетом → Wi-Fi или кабель → <b>Специальный</b>.</li>
<li>IP-адрес, DHCP, DNS и MTU: Автоматически / Не указывать.</li>
<li>Прокси-сервер → <b>Использовать</b>: адрес <code>{ip}</code>, порт <code>{port}</code>.</li></ol>
<div class="muted">Выключить: те же шаги, Прокси-сервер → Не использовать.</div></div>
<div class="card muted">Проверено не на всех приставках. Если что-то не заработало, напишите нам.</div>
</section>

<div class="card muted">Через прокси из настроек Wi-Fi звонки Discord и протокол QUIC не идут, YouTube и сайты при этом работают через обычное соединение. Звонки идут через v2rayNG или другой SOCKS5-клиент с UDP.</div>
</main><script>
function done(b){{var o=b.textContent;b.textContent='Скопировано';setTimeout(function(){{b.textContent=o}},1500)}}
function old(t,b){{var a=document.createElement('textarea');a.value=t;a.setAttribute('readonly','');a.style.position='fixed';a.style.top='0';a.style.opacity='0';document.body.appendChild(a);a.focus();a.select();a.setSelectionRange(0,t.length);var ok=false;try{{ok=document.execCommand('copy')}}catch(e){{}}document.body.removeChild(a);if(ok){{done(b)}}else{{var r=document.createRange();r.selectNodeContents(document.getElementById(b.dataset.id));var s=window.getSelection();s.removeAllRanges();s.addRange(r);b.textContent='Выделено, скопируйте'}}}}
function cp(id,b){{b.dataset.id=id;var t=document.getElementById(id).innerText.trim();if(navigator.clipboard&&window.isSecureContext){{navigator.clipboard.writeText(t).then(function(){{done(b)}},function(){{old(t,b)}})}}else{{old(t,b)}}}}
function show(d){{var e=document.querySelectorAll('[data-d]');for(var i=0;i<e.length;i++){{e[i].classList.toggle('on',e[i].getAttribute('data-d')===d)}}}}
var tb=document.querySelectorAll('.tabs button');for(var i=0;i<tb.length;i++){{tb[i].onclick=function(){{show(this.getAttribute('data-d'))}}}}
var ua=navigator.userAgent;
show(/PlayStation/i.test(ua)?'ps':/Android TV|GoogleTV|BRAVIA|AFT|MiTV|SmartTV|\bTV\b/i.test(ua)?'tv':/iPhone|iPad|iPod/i.test(ua)?'iphone':'android');
</script></body></html>"#
    )
}

pub fn respond(path: &str, addr: &str, on: bool) -> Vec<u8> {
    let path = path.split('?').next().unwrap_or("/");
    let (status, ctype, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", html(addr, on)),
        "/proxy.pac" | "/wpad.dat" => ("200 OK", "application/x-ns-proxy-autoconfig", pac(addr, on)),
        _ => ("404 Not Found", "text/plain; charset=utf-8", "Не найдено".to_string()),
    };
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())
        .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pac_and_page() {
        assert_eq!(pac("192.168.1.5:8881", false), "function FindProxyForURL(url, host) {\n  return \"DIRECT\";\n}\n");
        assert!(html("192.168.1.5:8881", false).contains("Раздача сейчас выключена"));
        let p = pac("192.168.1.5:8881", true);
        assert!(p.contains("return \"PROXY 192.168.1.5:8881; DIRECT\";"));
        assert!(p.contains("isInNet(ip, \"192.168.0.0\", \"255.255.0.0\")"));
        let h = html("192.168.1.5:8881", true);
        assert!(h.contains("http://192.168.1.5:8881/proxy.pac") && h.contains("<code id=\"port\">8881</code>") && h.contains("function cp(id,b)"));
        for d in ["android", "iphone", "tv", "ps"] {
            assert_eq!(h.matches(&format!("data-d=\"{d}\"")).count(), 2, "{d}");
        }
        assert!(h.contains("<code id=\"tip\">192.168.1.5</code>") && h.contains("<code id=\"pport\">8881</code>"));
        assert!(h.contains("<code id=\"v2\">socks://192.168.1.5:8881#HolyZapret</code>"));
        let r = String::from_utf8(respond("/proxy.pac?x=1", "10.0.0.2:8881", true)).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig"));
        assert!(String::from_utf8(respond("/x", "a:1", true)).unwrap().starts_with("HTTP/1.1 404"));
    }
}

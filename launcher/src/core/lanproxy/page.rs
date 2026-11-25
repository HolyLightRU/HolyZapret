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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gw {
    pub on: bool,
    pub ip: String,
    pub mask: String,
    pub prefix: u8,
}

const CSS: &str = r#"body{margin:0;font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;background:#141018;color:#eee;line-height:1.5}
main{max-width:560px;margin:0 auto;padding:20px 16px 40px}
h1{font-size:22px;margin:4px 0 6px;background:linear-gradient(90deg,#f28fc9,#69cdf5);-webkit-background-clip:text;background-clip:text;color:transparent}
.card{background:#201b27;border:1px solid #2e2836;border-radius:14px;padding:14px 16px;margin:14px 0}
.addr{font-size:22px;font-weight:600;letter-spacing:.3px}
code{background:#2b2433;border-radius:6px;padding:2px 6px;font-size:15px;overflow-wrap:anywhere}
ol{padding-left:20px;margin:6px 0}li{margin:6px 0}
.muted{color:#a59fae;font-size:14px}
.warn{background:#3a2a12;border-color:#7a5a20;color:#f6d79a}
.warn b{color:#ffe2a8}
.good{background:#14291f;border-color:#2f6a4a}
button,.btn{background:linear-gradient(90deg,#f28fc9,#69cdf5);border:0;border-radius:10px;padding:10px 14px;font-size:15px;color:#24122a;font-weight:600;text-decoration:none;display:inline-block}
.btn{margin:10px 0 4px}
.tabs{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0 0}
.tabs button{background:#2b2433;color:#ddd;font-weight:500;padding:8px 12px}
.tabs button.on{background:linear-gradient(90deg,#f28fc9,#69cdf5);color:#24122a;font-weight:600}
section{display:none}section.on{display:block}
#v2{display:block;margin:6px 0;font-size:13px}
table.kv{width:100%;border-collapse:collapse;margin:6px 0}
table.kv tr{display:block;padding:8px 0;border-bottom:1px solid #2e2836}
table.kv tr:last-child{border-bottom:0}
table.kv td{display:block;padding:0}
table.kv td:first-child{color:#a59fae;font-size:13px}
table.kv td:last-child{display:flex;align-items:center;justify-content:space-between;gap:10px;margin-top:2px}
table.kv code{font-size:17px}
table.kv button{padding:6px 12px;font-size:13px;flex:none}
a.back{color:#69cdf5}"#;

const JS: &str = r#"function done(b){var o=b.textContent;b.textContent='Скопировано';setTimeout(function(){b.textContent=o},1500)}
function old(t,b){var a=document.createElement('textarea');a.value=t;a.setAttribute('readonly','');a.style.position='fixed';a.style.top='0';a.style.opacity='0';document.body.appendChild(a);a.focus();a.select();a.setSelectionRange(0,t.length);var ok=false;try{ok=document.execCommand('copy')}catch(e){}document.body.removeChild(a);if(ok){done(b)}else{var r=document.createRange();r.selectNodeContents(document.getElementById(b.dataset.id));var s=window.getSelection();s.removeAllRanges();s.addRange(r);b.textContent='Выделено, скопируйте'}}
function cp(id,b){b.dataset.id=id;var t=document.getElementById(id).innerText.trim();if(navigator.clipboard&&window.isSecureContext){navigator.clipboard.writeText(t).then(function(){done(b)},function(){old(t,b)})}else{old(t,b)}}
function show(d){var e=document.querySelectorAll('[data-d]');for(var i=0;i<e.length;i++){e[i].classList.toggle('on',e[i].getAttribute('data-d')===d)}}
var tb=document.querySelectorAll('.tabs button');for(var i=0;i<tb.length;i++){tb[i].onclick=function(){show(this.getAttribute('data-d'))}}
var ua=navigator.userAgent;
var byUa=/PlayStation 4/i.test(ua)?'ps4':/PlayStation/i.test(ua)?'ps':/Xbox/i.test(ua)?'xbox':/Nintendo/i.test(ua)?'switch':/Android TV|GoogleTV|BRAVIA|AFT|MiTV|SmartTV|Tizen|Web0S|webOS|\bTV\b/i.test(ua)?'tv':/iPhone|iPad|iPod/i.test(ua)?'iphone':'android';
var h=location.hash.slice(1);
function has(d){return document.querySelector('section[data-d="'+d+'"]')}
show(h&&has(h)?h:has(byUa)?byUa:(byUa==='ps4'&&has('ps')?'ps':document.querySelector('.tabs button').getAttribute('data-d')));"#;

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"ru\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{title}</title>\n<style>\n{CSS}\n</style></head><body><main>\n{body}\n</main><script>\n{JS}\n</script></body></html>"
    )
}

fn copy_row(key: &str, id: &str, value: &str) -> String {
    format!("<tr><td>{key}</td><td><code id=\"{id}\">{value}</code><button onclick=\"cp('{id}',this)\">Копировать</button></td></tr>")
}

pub fn gw_values(gw: &Gw, id: &str) -> String {
    format!(
        "<table class=\"kv\"><tr><td>IP-адрес</td><td>тот, что устройство получило само</td></tr>{}{}<tr><td>Длина префикса</td><td><code>{}</code> (спрашивают телевизоры с Android)</td></tr>{}{}<tr><td>Прокси</td><td>не использовать</td></tr></table>",
        copy_row("Маска подсети", &format!("{id}m"), &gw.mask),
        copy_row("Шлюз (на телефонах Android — «Роутер»)", &format!("{id}g"), &gw.ip),
        gw.prefix,
        copy_row("Основной DNS", &format!("{id}d1"), "8.8.8.8"),
        copy_row("Дополнительный DNS", &format!("{id}d2"), "1.1.1.1"),
    )
}

fn gw_off_note(gw: &Gw) -> &'static str {
    if gw.on {
        ""
    } else {
        "<div class=\"card warn\"><b>Сначала включите режим на компьютере.</b> В HolyZapret на компьютере: вкладка «Телефон» → «Компьютер как шлюз». Пока он выключен, устройство с такими настройками выходит в интернет без обхода.</div>"
    }
}

pub fn html(addr: &str, on: bool, gw: &Gw, tg: Option<&str>) -> String {
    let (ip, port) = addr.rsplit_once(':').unwrap_or((addr, ""));
    let link = v2ray_link(addr);
    let state = if on {
        ""
    } else {
        r#"<div class="card warn"><b>Раздача сейчас выключена.</b> Если на устройстве выбран прокси вручную, уберите его, иначе интернет на этой сети не будет работать.</div>"#
    };
    let tg_card = match tg {
        Some(l) => format!(
            "<div class=\"card good\"><b>Telegram без VPN</b><div class=\"muted\">Telegram на этом телефоне может ходить через прокси компьютера, отдельно от всего остального. Нажмите кнопку, Telegram предложит подключить прокси.</div><a class=\"btn\" href=\"{l}\">Подключить прокси в Telegram</a></div>"
        ),
        None => String::new(),
    };
    let gw_state = if gw.on { "<span class=\"muted\">Сейчас включён на компьютере.</span>" } else { "<span class=\"muted\">Сначала включите его в HolyZapret на компьютере: вкладка «Телефон» → «Компьютер как шлюз».</span>" };
    let body = format!(
        r#"<h1>Обход HolyZapret на других устройствах</h1>
<div class="muted">Устройство пойдёт в интернет через компьютер с HolyZapret. Выберите, что настраиваете:</div>
<div class="tabs"><button data-d="android">Android</button><button data-d="iphone">iPhone</button><button data-d="tv">Телевизор</button><button data-d="ps">PlayStation</button><button data-d="xbox">Xbox и Switch</button></div>
{state}
<div class="card"><div class="muted">Адрес прокси</div><div class="addr">{ip} : {port}</div></div>

<section data-d="android">
{tg_card}
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
{tg_card}
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
<div class="card"><b>Способ 1. Прокси: Android TV и Google TV (Xiaomi Mi TV, Sony, TCL, Philips, приставки Mi Box)</b>
<ol><li>Настройки → Сеть и Интернет.</li>
<li>Выберите вашу Wi-Fi сеть. На некоторых моделях: удерживайте её или откройте «Дополнительно».</li>
<li>Настройки прокси-сервера → <b>Вручную</b>.</li>
<li>Имя хоста прокси: <code id="tip">{ip}</code></li>
<li>Порт: <code id="tport">{port}</code></li>
<li>Строку «Обходить прокси для» оставьте пустой и сохраните.</li></ol>
<div class="muted">Если телевизор подключён кабелем, прокси ищите в настройках Ethernet. Набирать адрес пультом долго, удобнее подключить к телевизору телефон как пульт через приложение Google TV.</div></div>
<div class="card"><b>Способ 2. Компьютер как шлюз</b> — если YouTube через прокси не заработал или у телевизора нет настройки прокси (Samsung, LG). Через компьютер пойдёт всё, что открывает телевизор. {gw_state}
<br><a class="btn" href="/gateway#tv">Пошагово для телевизора</a></div>
<div class="card muted">Проверено не на всех моделях. Если YouTube на телевизоре не заработал, напишите нам модель телевизора.</div>
</section>

<section data-d="ps">
<div class="card good"><b>Способ 1. Компьютер как шлюз (рекомендуем)</b>
<div class="muted">Через компьютер пойдёт всё, что открывает приставка, в том числе приложение YouTube: через прокси оно не работает. {gw_state}</div>
<a class="btn" href="/gateway#ps">Пошагово для PlayStation 5</a> <a class="btn" href="/gateway#ps4">Для PlayStation 4</a></div>
<div class="card"><b>Способ 2. Прокси: браузер и магазин</b>
<div class="muted">Приложение YouTube через прокси не работает, только браузер приставки. Онлайн в играх может работать хуже: игры обычно ходят мимо прокси.</div>
<ol><li>PS5: Настройки → Сеть → Настройки → Настроить соединение с Интернетом. Наведите на вашу сеть, нажмите Options → Дополнительные настройки.</li>
<li>Прокси-сервер → <b>Использовать</b>. Адрес: <code id="pip">{ip}</code>, порт: <code id="pport">{port}</code>. Нажмите OK.</li></ol>
<div class="muted">PS4: Настройки → Сеть → Установить соединение с Интернетом → Wi-Fi или кабель → «Специальный», IP, DHCP, DNS и MTU — автоматически, прокси-сервер — «Использовать» с тем же адресом. Выключить: Прокси-сервер → «Не использовать».</div></div>
<div class="card muted">Проверено не на всех приставках. Если что-то не заработало, напишите нам.</div>
</section>

<section data-d="xbox">
<div class="card"><b>Xbox и Nintendo Switch: компьютер как шлюз</b>
<div class="muted">Прокси у этих приставок либо нет, либо его не используют приложения, поэтому подходит только этот способ. {gw_state}</div>
<a class="btn" href="/gateway#xbox">Пошагово для Xbox</a> <a class="btn" href="/gateway#switch">Для Nintendo Switch</a></div>
</section>

<div class="card muted">Через прокси из настроек Wi-Fi звонки Discord и протокол QUIC не идут, YouTube и сайты при этом работают через обычное соединение. Звонки идут через v2rayNG или другой SOCKS5-клиент с UDP.</div>"#
    );
    page("HolyZapret: обход на телефоне и ТВ", &body)
}

pub fn gateway_html(gw: &Gw) -> String {
    let off = gw_off_note(gw);
    let values = gw_values(gw, "v");
    let body = format!(
        r#"<h1>Компьютер как шлюз</h1>
<div class="muted">Устройство выходит в интернет через компьютер с HolyZapret, и обход получает всё, что оно открывает: приложения, игры, видео. Прокси на устройстве не нужен.</div>
{off}
<div class="card"><b>Что вписать в настройки сети устройства</b>
{values}
<div class="muted">IP-адрес оставьте тот, что устройство получило само: обычно он виден в состоянии сети.</div></div>
<div class="tabs"><button data-d="ps">PlayStation 5</button><button data-d="ps4">PlayStation 4</button><button data-d="xbox">Xbox</button><button data-d="switch">Switch</button><button data-d="tv">Телевизор</button><button data-d="android">Телефон Android</button></div>

<section data-d="ps"><div class="card"><b>PlayStation 5</b>
<ol><li>Узнайте текущий IP: Настройки → Сеть → Состояние соединения → Посмотреть состояние соединения.</li>
<li>Настройки → Сеть → Настройки → Настроить соединение с Интернетом.</li>
<li>Наведите на вашу сеть, нажмите Options → <b>Дополнительные настройки</b>.</li>
<li>Настройки IP-адреса → <b>Вручную</b>: «Адрес IPv4» (текущий адрес приставки), маска подсети и шлюз по умолчанию из таблицы выше.</li>
<li>Основной и дополнительный DNS — из таблицы.</li>
<li>Прокси-сервер → <b>Не использовать</b>. Нажмите OK.</li></ol>
<div class="muted">Вернуть как было: Настройки IP-адреса → «Автоматически».</div></div></section>

<section data-d="ps4"><div class="card"><b>PlayStation 4</b>
<ol><li>Настройки → Сеть → Установить соединение с Интернетом → Wi-Fi или кабель → <b>Специальный</b>.</li>
<li>Настройки IP-адреса → <b>Вручную</b>: IP-адрес, маска подсети, шлюз по умолчанию, основной и дополнительный DNS из таблицы.</li>
<li>Настройки MTU → Автоматически. Прокси-сервер → <b>Не использовать</b>.</li></ol>
<div class="muted">Вернуть как было: те же шаги, «Простой».</div></div></section>

<section data-d="xbox"><div class="card"><b>Xbox One и Series</b>
<ol><li>Настройки → Общие → Параметры сети → Дополнительные параметры.</li>
<li>Параметры IP → <b>Вручную</b>: IP-адрес, маска подсети и шлюз из таблицы.</li>
<li>Параметры DNS → <b>Вручную</b>: основной и дополнительный DNS из таблицы.</li></ol>
<div class="muted">Вернуть как было: Параметры IP и DNS → «Автоматически».</div></div></section>

<section data-d="switch"><div class="card"><b>Nintendo Switch</b>
<ol><li>Настройки системы → Интернет → Настройки Интернета.</li>
<li>Выберите вашу сеть → <b>Изменить настройки</b>.</li>
<li>Настройки IP-адреса → <b>Вручную</b>: IP-адрес, маска подсети и шлюз из таблицы.</li>
<li>Настройки DNS → <b>Вручную</b>: первичный и вторичный DNS из таблицы. Прокси → Нет. Сохраните.</li></ol>
<div class="muted">Вернуть как было: Настройки IP-адреса и DNS → «Автоматически».</div></div></section>

<section data-d="tv"><div class="card"><b>Android TV и Google TV</b>
<ol><li>Настройки → Сеть и Интернет → ваша сеть.</li>
<li>Настройки IP → <b>Статический</b>.</li>
<li>По очереди: IP-адрес, шлюз, длина префикса сети, DNS 1, DNS 2 — из таблицы.</li></ol>
<div class="muted">Samsung и LG: Настройки → Общие (или Подключение) → Сеть → Состояние сети → Настройки IP → «Ввести вручную», те же значения. Вернуть как было: «Получать автоматически».</div></div></section>

<section data-d="android"><div class="card"><b>Телефон Android</b>
<div class="muted">Для телефона обычно удобнее приложение v2rayNG (главная страница). Шлюз подходит, если приложение ставить не хочется.</div>
<ol><li>Настройки → Wi-Fi → нажмите на вашу сеть → Изменить (или шестерёнка) → Дополнительно.</li>
<li>Настройки IP → <b>Статический IP-адрес</b>.</li>
<li>IP-адрес оставьте как есть. В поле <b>«Роутер»</b> (или «Шлюз») впишите адрес шлюза из таблицы. Префикс — длина префикса, DNS 1 и DNS 2 — из таблицы.</li>
<li>Прокси → Нет. Сохраните.</li></ol>
<div class="muted">Вернуть как было: Настройки IP → DHCP. Telegram на телефоне через шлюз ходит напрямую и в РФ обычно не подключается: для него включите в HolyZapret Telegram-прокси «для телефонов в домашней сети».</div></div></section>

<div class="card warn"><b>Важно.</b> Компьютер с HolyZapret должен быть включён, иначе у устройства не будет интернета. Если роутер выдаст компьютеру другой адрес, шлюз на устройстве нужно поменять, поэтому адрес компьютера лучше закрепить в настройках роутера. Не работает в гостевой сети Wi-Fi и если роутер изолирует устройства друг от друга.</div>
<div class="muted"><a class="back" href="/">← Все способы</a></div>"#
    );
    page("HolyZapret: компьютер как шлюз", &body)
}

pub fn respond(path: &str, addr: &str, on: bool, gw: &Gw, tg: Option<&str>) -> Vec<u8> {
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let (status, ctype, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", html(addr, on, gw, tg)),
        "/gateway" => ("200 OK", "text/html; charset=utf-8", gateway_html(gw)),
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

    fn gw(on: bool) -> Gw {
        Gw { on, ip: "192.168.1.5".into(), mask: "255.255.255.0".into(), prefix: 24 }
    }

    #[test]
    fn pac_and_page() {
        assert_eq!(pac("192.168.1.5:8881", false), "function FindProxyForURL(url, host) {\n  return \"DIRECT\";\n}\n");
        assert!(html("192.168.1.5:8881", false, &gw(false), None).contains("Раздача сейчас выключена"));
        let p = pac("192.168.1.5:8881", true);
        assert!(p.contains("return \"PROXY 192.168.1.5:8881; DIRECT\";"));
        assert!(p.contains("isInNet(ip, \"192.168.0.0\", \"255.255.0.0\")"));
        let h = html("192.168.1.5:8881", true, &gw(true), None);
        assert!(!h.contains("Telegram без VPN"));
        let t = html("192.168.1.5:8881", true, &gw(true), Some("https://t.me/proxy?server=192.168.1.5&port=1443&secret=dd00"));
        assert_eq!(t.matches("href=\"https://t.me/proxy?server=192.168.1.5").count(), 2);
        assert!(h.contains("http://192.168.1.5:8881/proxy.pac") && h.contains("<code id=\"port\">8881</code>") && h.contains("function cp(id,b)"));
        for d in ["android", "iphone", "tv", "ps", "xbox"] {
            assert_eq!(h.matches(&format!("data-d=\"{d}\"")).count(), 2, "{d}");
        }
        assert!(h.contains("<code id=\"tip\">192.168.1.5</code>") && h.contains("<code id=\"pport\">8881</code>"));
        assert!(h.contains("<code id=\"v2\">socks://192.168.1.5:8881#HolyZapret</code>"));
        assert!(h.contains("href=\"/gateway#ps\"") && h.contains("Сейчас включён на компьютере"));
        assert!(["{ip}", "{port}", "{gw_state}", "{state}"].iter().all(|x| !h.contains(x)));
        let r = String::from_utf8(respond("/proxy.pac?x=1", "10.0.0.2:8881", true, &gw(true), None)).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig"));
        assert!(String::from_utf8(respond("/x", "a:1", true, &gw(true), None)).unwrap().starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn gateway_page_has_values_and_devices() {
        let h = gateway_html(&gw(true));
        assert!(h.contains("<code id=\"vg\">192.168.1.5</code>") && h.contains("<code id=\"vm\">255.255.255.0</code>"));
        assert!(h.contains("<code>24</code>") && h.contains("<code id=\"vd1\">8.8.8.8</code>"));
        for d in ["ps", "ps4", "xbox", "switch", "tv", "android"] {
            assert_eq!(h.matches(&format!("data-d=\"{d}\"")).count(), 2, "{d}");
        }
        assert!(!h.contains("Сначала включите режим"));
        assert!(gateway_html(&gw(false)).contains("Сначала включите режим"));
        let r = String::from_utf8(respond("/gateway", "192.168.1.5:8881", false, &gw(true), None)).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK") && r.contains("Компьютер как шлюз"));
    }
}

//! U-a（WebView2）の画面の HTML。固定のデータ（`model.rs`）から作る。OS に依存しない。
//! 変更は `window.ipc.postMessage("change\t<key>\t<value>")` で Rust に渡る。
//! 最初の描画のあと（requestAnimationFrame を 2 回待つ）に `"ready"` を送る。

use crate::model::{Item, Kind};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn control(item: &Item) -> String {
    let key = esc(item.key);
    let init = esc(item.initial);
    match &item.kind {
        Kind::Bool => {
            let checked = if item.initial == "true" {
                " checked"
            } else {
                ""
            };
            format!(
                r#"<label class="switch"><input type="checkbox" role="switch" data-key="{key}"{checked}><span class="track"></span></label>"#
            )
        }
        Kind::Int { min, max } => format!(
            r#"<span class="int"><input type="range" min="{min}" max="{max}" value="{init}" data-key="{key}" data-int="1" aria-label="{label}"><input type="number" min="{min}" max="{max}" value="{init}" data-key="{key}" data-int="1" aria-label="{label}"></span>"#,
            label = esc(item.label)
        ),
        Kind::Enum { choices } => {
            let opts: String = choices
                .iter()
                .map(|c| {
                    let sel = if *c == item.initial { " selected" } else { "" };
                    format!(r#"<option value="{c}"{sel}>{c}</option>"#)
                })
                .collect();
            format!(
                r#"<select data-key="{key}" aria-label="{label}">{opts}</select>"#,
                label = esc(item.label)
            )
        }
        // lang="ja" は IME の選択の手がかり。通常の input なので、Windows の標準の IME の扱いになる見込み
        Kind::Text => format!(
            r#"<input type="text" lang="ja" value="{init}" data-key="{key}" data-text="1" aria-label="{label}" placeholder="ここに日本語を入力">"#,
            label = esc(item.label)
        ),
        Kind::KeyChord => format!(
            r#"<input type="text" readonly class="chord" value="{init}" data-key="{key}" data-chord="1" aria-label="{label}">"#,
            label = esc(item.label)
        ),
        Kind::BundleIdList => {
            let lis: String = item
                .initial
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|id| {
                    let id = esc(id);
                    format!(
                        r#"<li><span>{id}</span><button type="button" data-remove="1" aria-label="{id} を削除">削除</button></li>"#
                    )
                })
                .collect();
            format!(
                r#"<div class="ids" data-key="{key}" data-ids="1"><ul>{lis}</ul><div class="add"><input type="text" lang="ja" placeholder="追加する ID" aria-label="追加する ID"><button type="button" data-add="1">追加</button></div></div>"#
            )
        }
    }
}

pub fn page(items: &[Item]) -> String {
    let rows: String = items
        .iter()
        .map(|item| {
            format!(
                r#"<div class="row"><div class="text"><div class="label">{}</div><div class="desc">{}</div></div><div class="ctl">{}</div></div>"#,
                esc(item.label),
                esc(item.description),
                control(item)
            )
        })
        .collect();
    format!("{HEAD}{rows}{TAIL}")
}

const HEAD: &str = r#"<!doctype html><html lang="ja"><head><meta charset="utf-8">
<style>
:root{color-scheme:light dark;--bg:#f3f3f3;--card:#fff;--fg:#1b1b1b;--sub:#5d5d5d;--line:#e5e5e5;--accent:#0067c0}
@media (prefers-color-scheme:dark){:root{--bg:#202020;--card:#2b2b2b;--fg:#fff;--sub:#c5c5c5;--line:#3a3a3a;--accent:#4cc2ff}}
html,body{margin:0;background:var(--bg);color:var(--fg);font:14px "Segoe UI Variable Text","Segoe UI","Yu Gothic UI",sans-serif}
main{max-width:680px;margin:0 auto;padding:20px}
h1{font-size:20px;font-weight:600;margin:0 0 16px}
.row{display:flex;justify-content:space-between;align-items:center;gap:16px;background:var(--card);border:1px solid var(--line);border-radius:6px;padding:12px 16px;margin-bottom:6px}
.label{font-weight:500}.desc{color:var(--sub);font-size:12px;margin-top:2px}
.ctl{flex:0 0 auto;min-width:220px;display:flex;justify-content:flex-end}
.ctl input[type=text],.ctl select,.ctl input[type=number]{font:inherit;color:var(--fg);background:transparent;border:1px solid var(--line);border-bottom-color:var(--sub);border-radius:4px;padding:5px 8px}
.ctl input[type=text]{width:200px}.ctl input[type=number]{width:56px}
.int{display:flex;gap:8px;align-items:center}
.switch{position:relative;display:inline-block;width:40px;height:20px}
.switch input{position:absolute;inset:0;opacity:0;margin:0;cursor:pointer}
.track{position:absolute;inset:0;border:1px solid var(--sub);border-radius:10px;transition:.1s}
.track::after{content:"";position:absolute;top:4px;left:4px;width:10px;height:10px;border-radius:50%;background:var(--sub);transition:.1s}
.switch input:checked+.track{background:var(--accent);border-color:var(--accent)}
.switch input:checked+.track::after{left:24px;background:#fff}
.switch input:focus-visible+.track,.ctl :focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.ids{width:260px}.ids ul{list-style:none;margin:0 0 6px;padding:0}
.ids li{display:flex;justify-content:space-between;align-items:center;padding:3px 0}
.ids .add{display:flex;gap:6px}.ids .add input{flex:1;width:auto}
button{font:inherit;color:var(--fg);background:var(--card);border:1px solid var(--line);border-radius:4px;padding:4px 10px}
button:hover{background:var(--line)}
</style></head><body><main><h1>ikari-ime 設定（スパイク: WebView2）</h1>
"#;

const TAIL: &str = r#"</main><script>
function send(k,v){window.ipc.postMessage('change\t'+k+'\t'+v)}
document.addEventListener('change',function(e){
  var t=e.target,k=t.dataset.key;if(!k)return;
  if(t.type==='checkbox')send(k,t.checked?'true':'false');
  else if(t.dataset.int){
    var n=Math.max(+t.min,Math.min(+t.max,parseInt(t.value,10)||+t.min));
    document.querySelectorAll('[data-key="'+k+'"]').forEach(function(x){x.value=n});send(k,String(n));
  }else if(t.tagName==='SELECT')send(k,t.value);
});
// 文字列は、変換の確定前の途中の文字を送らない（compositionend のあとで送る）
var composing=false;
document.addEventListener('compositionstart',function(){composing=true});
document.addEventListener('compositionend',function(e){composing=false;if(e.target.dataset.text)send(e.target.dataset.key,e.target.value)});
document.addEventListener('input',function(e){var t=e.target;if(t.dataset.text&&!composing)send(t.dataset.key,t.value)});
// キー割り当て: 押したキーを "ctrl+j" の形にする。Backspace で none
document.addEventListener('keydown',function(e){
  var t=e.target;if(!t.dataset||!t.dataset.chord)return;
  if(e.key==='Tab')return;
  e.preventDefault();
  if(['Control','Shift','Alt','Meta'].indexOf(e.key)>=0)return;
  var v;
  if(e.key==='Backspace'&&!e.ctrlKey&&!e.altKey&&!e.shiftKey)v='none';
  else{var p=[];if(e.ctrlKey)p.push('ctrl');if(e.altKey)p.push('alt');if(e.shiftKey)p.push('shift');if(e.metaKey)p.push('win');
    p.push(e.key.length===1?e.key.toLowerCase():e.key.toLowerCase());v=p.join('+')}
  t.value=v;send(t.dataset.key,v);
});
// Bundle ID の一覧: 追加と削除
function ids(box){return Array.prototype.map.call(box.querySelectorAll('li span'),function(s){return s.textContent}).join(',')}
document.addEventListener('click',function(e){
  var b=e.target,box=b.closest&&b.closest('[data-ids]');if(!box)return;
  if(b.dataset.remove){b.closest('li').remove();send(box.dataset.key,ids(box))}
  else if(b.dataset.add){
    var inp=box.querySelector('.add input'),v=inp.value.trim();if(!v)return;
    var li=document.createElement('li'),s=document.createElement('span'),r=document.createElement('button');
    s.textContent=v;r.type='button';r.dataset.remove='1';r.textContent='削除';r.setAttribute('aria-label',v+' を削除');
    li.appendChild(s);li.appendChild(r);box.querySelector('ul').appendChild(li);inp.value='';send(box.dataset.key,ids(box));
  }
});
requestAnimationFrame(function(){requestAnimationFrame(function(){window.ipc.postMessage('ready')})});
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixed_items;

    #[test]
    fn page_has_every_key() {
        let html = page(&fixed_items());
        for item in fixed_items() {
            assert!(
                html.contains(&format!(r#"data-key="{}""#, item.key)),
                "{}",
                item.key
            );
        }
    }
}

//! 候補窓に出す固定の入力。`shell-core` の `CandidatePanelModel` / `CandidateItem` と同じ形
//! （surface、annotation、label、selected、annotation、reserved_rows）。

pub const PAGE_SIZE: usize = 5;

#[derive(Clone)]
pub struct Item {
    pub surface: &'static str,
    /// 2 行目に出す注釈（選択中の行だけに出す）
    pub annotation: &'static str,
    /// 行の右に薄く出す名前
    pub label: &'static str,
}

#[derive(Clone)]
pub struct Panel {
    pub candidates: Vec<Item>,
    pub selected: usize,
    /// 注釈の行。空なら出さない
    pub annotation: &'static str,
    pub reserved_rows: usize,
}

impl Panel {
    /// 9 件、2 ページ（5 件 + 4 件）、注釈つき
    pub fn fixed() -> Self {
        let item = |surface, annotation, label| Item {
            surface,
            annotation,
            label,
        };
        Self {
            candidates: vec![
                item("橋", "はし。bridge", ""),
                item("箸", "はし。chopsticks", ""),
                item("端", "はし。edge", ""),
                item("嘴", "くちばし", ""),
                item("梯", "はしご", ""),
                item("ハシ", "カタカナ", ""),
                item("はし", "ひらがな", ""),
                item("HASHI", "", ""),
                item("👀", "目", ":eyes:"),
            ],
            selected: 0,
            annotation: "辞書: SudachiDict Core",
            reserved_rows: PAGE_SIZE,
        }
    }

    pub fn page_count(&self) -> usize {
        self.candidates.len().div_ceil(PAGE_SIZE)
    }

    pub fn page(&self) -> usize {
        self.selected / PAGE_SIZE
    }

    /// 今のページの (番号 1..=5, 候補, 選択中か)
    pub fn rows(&self) -> Vec<(usize, &Item, bool)> {
        let start = self.page() * PAGE_SIZE;
        self.candidates
            .iter()
            .enumerate()
            .skip(start)
            .take(PAGE_SIZE)
            .map(|(i, c)| (i - start + 1, c, i == self.selected))
            .collect()
    }

    pub fn footer(&self) -> String {
        let mut s = format!("{}/{}", self.page() + 1, self.page_count());
        if !self.annotation.is_empty() {
            s.push_str("  ");
            s.push_str(self.annotation);
        }
        s
    }

    /// 次の候補へ（最後の次は先頭）
    pub fn advance(&mut self) {
        self.selected = (self.selected + 1) % self.candidates.len();
    }
}

pub const HTML_ROW_PX: u32 = 28;
pub const HTML_FOOTER_PX: u32 = 24;
pub const HTML_WIDTH_PX: u32 = 300;

pub fn html_height(rows: usize) -> u32 {
    rows as u32 * HTML_ROW_PX + HTML_FOOTER_PX + 2
}

/// 方式 A の中身。JS の `setSelected(n)` で選択を替え、描き終わったら `window.ipc` に知らせる。
/// 文字列はすべてこのファイルの固定値なので、エスケープはしない（スパイクのみ）。
pub fn html(p: &Panel) -> String {
    let items: Vec<String> = p
        .candidates
        .iter()
        .map(|c| {
            format!(
                "{{s:\"{}\",a:\"{}\",l:\"{}\"}}",
                c.surface, c.annotation, c.label
            )
        })
        .collect();
    format!(
        r#"<!doctype html><meta charset="utf-8"><style>
html,body{{margin:0;background:#fff;font:15px "Yu Gothic UI",sans-serif;overflow:hidden;user-select:none}}
.row{{height:{row}px;line-height:{row}px;padding:0 8px;display:flex;gap:8px;white-space:nowrap}}
.row.sel{{background:#cfe3ff}} .n{{color:#666;width:1.2em}} .l{{color:#999;margin-left:auto}}
.a{{color:#666;font-size:12px}} #f{{height:{foot}px;line-height:{foot}px;padding:0 8px;color:#666;font-size:12px;border-top:1px solid #ddd}}
</style><div id="rows"></div><div id="f"></div>
<script>
const C=[{items}], PS={ps}, ANN="{ann}";
function setSelected(sel){{
  const start=Math.floor(sel/PS)*PS; let h="";
  for(let i=start;i<Math.min(start+PS,C.length);i++){{
    const c=C[i], cls=i===sel?"row sel":"row";
    h+='<div class="'+cls+'"><span class="n">'+(i-start+1)+'</span><span>'+c.s+'</span>'+
       (i===sel&&c.a?'<span class="a">'+c.a+'</span>':'')+'<span class="l">'+c.l+'</span></div>';
  }}
  document.getElementById("rows").innerHTML=h;
  document.getElementById("f").textContent=(Math.floor(sel/PS)+1)+"/"+Math.ceil(C.length/PS)+(ANN?"  "+ANN:"");
  requestAnimationFrame(()=>requestAnimationFrame(()=>window.ipc.postMessage("painted:"+sel)));
}}
setSelected({sel});
</script>"#,
        row = HTML_ROW_PX,
        foot = HTML_FOOTER_PX,
        items = items.join(","),
        ps = PAGE_SIZE,
        ann = p.annotation,
        sel = p.selected,
    )
}

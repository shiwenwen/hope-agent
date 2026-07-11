//! 设计模板（Recipe）骨架 demo——工具箱 hover 预览用的**纯形状 wireframe** 自包含 HTML。
//!
//! 统一 900×620 虚拟画布（前端 iframe 等比缩放零适配）；无文字（文字行用圆角条模拟，
//! 零 i18n / 零 slop），配色全走 `var(--ds-*)`——**吃当前会话选中设计系统的 tokens**，
//! 同一模板在不同设计系统下即时呈现对应配色气质。token 注入复用
//! [`renderer::tokens_root_css`]（与产物 / Kit 同一安全过滤）。motion 类骨架带真 CSS
//! 循环动画。零编译、零网络（沙箱 iframe 直渲）。

use std::collections::BTreeMap;

use super::renderer::tokens_root_css;

/// 统一画布尺寸（前端按宽等比缩放）。
pub const DEMO_CANVAS_W: u32 = 900;
pub const DEMO_CANVAS_H: u32 = 620;

// ── 布局原语（纯形状） ─────────────────────────────────────────────

/// 圆角条（模拟一行文字）。`cls`: strong（标题）/ soft（正文）/ inv（反白）/ pri（主色）。
fn bar(w: &str, h: u32, cls: &str) -> String {
    format!(r#"<i class="bar {cls}" style="width:{w};height:{h}px"></i>"#)
}

/// 主按钮 / 幽灵按钮块。
fn btn(w: u32) -> String {
    format!(r#"<i class="btn" style="width:{w}px;height:34px"></i>"#)
}
fn ghost_btn(w: u32) -> String {
    format!(r#"<i class="ghostbtn" style="width:{w}px;height:34px"></i>"#)
}

/// 顶部导航条：logo 点 + 菜单条 ×4 + 主按钮。
fn nav() -> String {
    format!(
        r#"<div class="row" style="align-items:center;gap:14px;padding:16px 36px;border-bottom:1px solid var(--ds-color-border,#e4e7ec)">
<i class="chip" style="width:26px;height:26px;border-radius:8px;background:var(--ds-color-primary,#4f46e5)"></i>{}
<span style="flex:1"></span>{}{}{}{}{}</div>"#,
        bar("64px", 12, "strong"),
        bar("40px", 10, "soft"),
        bar("40px", 10, "soft"),
        bar("40px", 10, "soft"),
        bar("40px", 10, "soft"),
        btn(84),
    )
}

/// 特性卡：图标块 + 标题条 + 两行正文条。
fn feature_card() -> String {
    format!(
        r#"<div class="card col" style="flex:1;gap:10px;padding:18px">
<i class="chip" style="width:34px;height:34px;border-radius:10px;background:color-mix(in srgb,var(--ds-color-primary,#4f46e5) 16%,transparent)"></i>
{}{}{}</div>"#,
        bar("62%", 12, "strong"),
        bar("92%", 8, "soft"),
        bar("78%", 8, "soft"),
    )
}

/// KPI 卡：小标签条 + 大数字条（+ 可选涨跌点）。
fn kpi_card(trend: bool) -> String {
    let dot = if trend {
        r#"<i class="chip" style="width:26px;height:10px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-success,#16a34a) 30%,transparent)"></i>"#
    } else {
        ""
    };
    format!(
        r#"<div class="card col" style="flex:1;gap:9px;padding:14px 16px">
<div class="row" style="justify-content:space-between;align-items:center">{}{dot}</div>{}</div>"#,
        bar("46%", 8, "soft"),
        bar("58%", 18, "strong"),
    )
}

/// 内联 SVG 折线图（stroke 主色）。
fn chart_line(w: u32, h: u32) -> String {
    format!(
        r#"<svg width="{w}" height="{h}" viewBox="0 0 {w} {h}" style="display:block">
<polyline points="0,{y0} {x1},{y1} {x2},{y2} {x3},{y3} {x4},{y4} {w},{y5}" fill="none" stroke="var(--ds-color-primary,#4f46e5)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>
<line x1="0" y1="{base}" x2="{w}" y2="{base}" stroke="var(--ds-color-border,#e4e7ec)" stroke-width="1.5"/></svg>"#,
        y0 = h * 3 / 4,
        x1 = w / 5,
        y1 = h / 2,
        x2 = w * 2 / 5,
        y2 = h * 3 / 5,
        x3 = w * 3 / 5,
        y3 = h / 3,
        x4 = w * 4 / 5,
        y4 = h * 2 / 5,
        y5 = h / 6,
        base = h - 1,
    )
}

/// 内联 SVG 柱状图。
fn chart_bars(w: u32, h: u32) -> String {
    let n = 6u32;
    let bw = w / (n * 2);
    let heights = [40u32, 62, 48, 78, 58, 88];
    let mut bars = String::new();
    for (i, hp) in heights.iter().enumerate() {
        let bh = h * hp / 100;
        let x = bw / 2 + i as u32 * bw * 2;
        let fill = if i == 3 {
            "var(--ds-color-primary,#4f46e5)"
        } else {
            "color-mix(in srgb,var(--ds-color-primary,#4f46e5) 32%,transparent)"
        };
        bars.push_str(&format!(
            r#"<rect x="{x}" y="{y}" width="{bw}" height="{bh}" rx="3" fill="{fill}"/>"#,
            y = h - bh,
        ));
    }
    format!(
        r#"<svg width="{w}" height="{h}" viewBox="0 0 {w} {h}" style="display:block">{bars}</svg>"#
    )
}

/// 表格骨架：表头 + n 行。
fn table_rows(n: u32) -> String {
    let mut rows = String::from(
        r#"<div class="row" style="gap:16px;padding:10px 14px;background:var(--ds-color-muted,#f1f3f6);border-radius:8px 8px 0 0">"#,
    );
    for w in ["18%", "26%", "16%", "12%"] {
        rows.push_str(&bar(w, 9, "strong"));
    }
    rows.push_str("</div>");
    for i in 0..n {
        let border = if i + 1 == n {
            ""
        } else {
            "border-bottom:1px solid var(--ds-color-border,#e4e7ec);"
        };
        rows.push_str(&format!(
            r#"<div class="row" style="gap:16px;padding:11px 14px;{border}">{}{}{}{}</div>"#,
            bar("18%", 8, "soft"),
            bar("26%", 8, "soft"),
            bar("16%", 8, "soft"),
            bar("12%", 8, "soft"),
        ));
    }
    rows
}

/// 文档纸面：画布垫 muted 底，中央 640 宽纸面。
fn doc_page(inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;height:100%;background:var(--ds-color-muted,#eef0f3);padding:26px 0 0">
<div class="card col" style="width:640px;gap:14px;padding:34px 44px;border-radius:14px 14px 0 0">{inner}</div></div>"#
    )
}

/// 16:9 幻灯片：深浅随 bg，中央 780×440 slide 卡。
fn slide(inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;align-items:center;height:100%;background:var(--ds-color-muted,#eef0f3)">
<div class="card" style="width:780px;height:440px;border-radius:14px;overflow:hidden;position:relative">{inner}</div></div>"#
    )
}

/// 手机框：270×560 圆角壳居中。
fn phone(inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;align-items:center;height:100%;background:var(--ds-color-muted,#eef0f3)">
<div class="card col" style="width:270px;height:560px;border-radius:34px;border-width:6px;overflow:hidden">{inner}</div></div>"#
    )
}

/// 海报框：宽高定制居中。
fn poster_frame(w: u32, h: u32, inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;align-items:center;height:100%;background:var(--ds-color-muted,#eef0f3)">
<div class="card col" style="width:{w}px;height:{h}px;border-radius:10px;overflow:hidden;position:relative">{inner}</div></div>"#
    )
}

/// 600 宽邮件卡居中。
fn email_frame(inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;height:100%;background:var(--ds-color-muted,#eef0f3);padding:26px 0 0">
<div class="card col" style="width:560px;border-radius:12px 12px 0 0;overflow:hidden">{inner}</div></div>"#
    )
}

// ── 25 个 recipe 骨架 ──────────────────────────────────────────────

fn demo_body(recipe_id: &str) -> Option<String> {
    let html = match recipe_id {
        "web-landing" => format!(
            r#"{nav}
<div class="col" style="align-items:center;gap:12px;padding:48px 0 34px">
{t1}{t2}<div style="height:2px"></div>{sub}
<div class="row" style="gap:12px;margin-top:14px">{b1}{b2}</div></div>
<div class="row" style="gap:18px;padding:0 60px">{c}{c}{c}</div>
<div class="row mutedcard" style="justify-content:center;align-items:center;gap:16px;margin:30px 60px 0;padding:22px">{s}{btn}</div>"#,
            nav = nav(),
            t1 = bar("420px", 24, "strong"),
            t2 = bar("300px", 24, "strong"),
            sub = bar("460px", 11, "soft"),
            b1 = btn(126),
            b2 = ghost_btn(104),
            c = feature_card(),
            s = bar("220px", 10, "soft"),
            btn = btn(110),
        ),
        "web-saas" => format!(
            r#"{nav}
<div class="row" style="gap:40px;padding:44px 60px;align-items:center">
<div class="col" style="flex:1.1;gap:12px">{t1}{t2}{sub}{sub2}<div class="row" style="gap:12px;margin-top:12px">{b}{g}</div></div>
<div class="mutedcard" style="flex:1;height:190px;border-radius:14px;position:relative;overflow:hidden">
<i class="chip" style="position:absolute;left:18px;top:18px;width:52%;height:12px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-primary,#4f46e5) 34%,transparent)"></i>
<i class="chip" style="position:absolute;left:18px;top:44px;width:70%;height:80px;border-radius:10px;background:color-mix(in srgb,var(--ds-color-fg,#16181d) 8%,transparent)"></i></div></div>
<div class="row" style="gap:16px;padding:0 60px">{p1}{p2}{p3}</div>"#,
            nav = nav(),
            t1 = bar("86%", 22, "strong"),
            t2 = bar("62%", 22, "strong"),
            sub = bar("92%", 10, "soft"),
            sub2 = bar("74%", 10, "soft"),
            b = btn(120),
            g = ghost_btn(96),
            p1 = pricing_card(false),
            p2 = pricing_card(true),
            p3 = pricing_card(false),
        ),
        "web-editorial" => format!(
            r#"<div class="col" style="align-items:center;gap:13px;padding:52px 0 0">
{kick}{t1}{t2}
<div class="row" style="gap:10px;margin:2px 0 16px">{m}{m}{m}</div>
<div class="col" style="width:560px;gap:11px">
{p}{p}{p2}<div style="height:6px"></div>
<div class="row" style="gap:14px;border-left:3px solid var(--ds-color-primary,#4f46e5);padding-left:16px">{quote}</div>
<div style="height:6px"></div>{p}{p}{p2}{p}</div></div>"#,
            kick = bar("90px", 10, "pri"),
            t1 = bar("480px", 26, "strong"),
            t2 = bar("360px", 26, "strong"),
            m = bar("64px", 9, "soft"),
            p = bar("100%", 10, "soft"),
            p2 = bar("82%", 10, "soft"),
            quote = bar("86%", 12, "strong"),
        ),
        "mobile-onboarding" => phone(&format!(
            r#"<div class="mutedcard" style="margin:16px 14px 0;height:250px;border-radius:18px;position:relative;overflow:hidden">
<i class="chip" style="position:absolute;inset:0;background:linear-gradient(135deg,color-mix(in srgb,var(--ds-color-primary,#4f46e5) 46%,transparent),color-mix(in srgb,var(--ds-color-accent,var(--ds-color-primary,#4f46e5)) 20%,transparent))"></i></div>
<div class="col" style="align-items:center;gap:10px;padding:24px 22px 0">{t1}{t2}{sub}{sub2}</div>
<span style="flex:1"></span>
<div class="row" style="justify-content:center;gap:6px;padding-bottom:14px">
<i class="chip" style="width:16px;height:6px;border-radius:99px;background:var(--ds-color-primary,#4f46e5)"></i>
<i class="chip" style="width:6px;height:6px;border-radius:99px;background:var(--ds-color-border,#dbe0e6)"></i>
<i class="chip" style="width:6px;height:6px;border-radius:99px;background:var(--ds-color-border,#dbe0e6)"></i></div>
<div class="col" style="padding:0 18px 20px">{b}</div>"#,
            t1 = bar("150px", 15, "strong"),
            t2 = bar("110px", 15, "strong"),
            sub = bar("180px", 8, "soft"),
            sub2 = bar("150px", 8, "soft"),
            b = r#"<i class="btn" style="width:100%;height:40px;border-radius:12px"></i>"#,
        )),
        "mobile-app" => phone(&format!(
            r#"<div class="row" style="align-items:center;gap:10px;padding:16px 16px 10px">{title}<span style="flex:1"></span>
<i class="chip" style="width:24px;height:24px;border-radius:99px;background:var(--ds-color-muted,#eef0f3)"></i></div>
<div class="col" style="gap:10px;padding:4px 14px">{card}{card}{card}{card}</div>
<span style="flex:1"></span>
<div class="row" style="justify-content:space-around;align-items:center;border-top:1px solid var(--ds-color-border,#e4e7ec);padding:12px 8px 16px">
{tab_on}{tab}{tab}{tab}{tab}</div>"#,
            title = bar("110px", 16, "strong"),
            card = r#"<div class="card row" style="gap:12px;padding:12px;align-items:center">
<i class="chip" style="width:40px;height:40px;border-radius:10px;background:color-mix(in srgb,var(--ds-color-primary,#4f46e5) 14%,transparent)"></i>
<div class="col" style="flex:1;gap:7px"><i class="bar strong" style="width:64%;height:10px"></i><i class="bar soft" style="width:88%;height:7px"></i></div></div>"#,
            tab_on = r#"<i class="chip" style="width:22px;height:22px;border-radius:7px;background:var(--ds-color-primary,#4f46e5)"></i>"#,
            tab = r#"<i class="chip" style="width:22px;height:22px;border-radius:7px;background:var(--ds-color-border,#dbe0e6)"></i>"#,
        )),
        "deck-pitch" => slide(&format!(
            r#"<i class="chip" style="position:absolute;right:-60px;top:-60px;width:240px;height:240px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-primary,#4f46e5) 14%,transparent)"></i>
<div class="col" style="position:absolute;left:52px;bottom:56px;gap:16px">{k}{t1}{t2}<div style="height:4px"></div>{m}</div>"#,
            k = bar("74px", 10, "pri"),
            t1 = bar("380px", 30, "strong"),
            t2 = bar("270px", 30, "strong"),
            m = bar("150px", 10, "soft"),
        )),
        "deck-report" => slide(&format!(
            r#"<div class="col" style="gap:12px;padding:40px 48px">{t}
<div class="row" style="gap:30px;margin-top:10px">
<div class="col" style="flex:1;gap:13px">{li}{li}{li}{li}</div>
<div class="mutedcard" style="flex:1.1;border-radius:12px;padding:18px">{chart}</div></div></div>"#,
            t = bar("46%", 20, "strong"),
            li = r#"<div class="row" style="gap:10px;align-items:center"><i class="chip" style="width:8px;height:8px;border-radius:99px;background:var(--ds-color-primary,#4f46e5)"></i><i class="bar soft" style="width:82%;height:10px"></i></div>"#,
            chart = chart_bars(300, 210),
        )),
        "deck-keynote" => slide(&format!(
            r#"<div class="col" style="align-items:center;justify-content:center;gap:20px;height:100%">{t1}{t2}{m}"#,
            t1 = bar("54%", 40, "strong"),
            t2 = bar("34%", 40, "pri"),
            m = bar("20%", 11, "soft"),
        )),
        "deck-timeline" => slide(&format!(
            r#"<div class="col" style="gap:26px;padding:44px 48px">{t}
<div style="position:relative;height:6px;background:var(--ds-color-muted,#eef0f3);border-radius:99px;margin-top:26px">
<i class="chip" style="position:absolute;left:0;top:0;width:46%;height:6px;border-radius:99px;background:var(--ds-color-primary,#4f46e5)"></i>
{d1}{d2}{d3}{d4}</div>
<div class="row" style="gap:18px;margin-top:22px">{ph}{ph}{ph}{ph}</div></div>"#,
            t = bar("40%", 20, "strong"),
            d1 = timeline_dot(6, true),
            d2 = timeline_dot(32, true),
            d3 = timeline_dot(60, false),
            d4 = timeline_dot(88, false),
            ph = r#"<div class="col" style="flex:1;gap:8px"><i class="bar strong" style="width:70%;height:11px"></i><i class="bar soft" style="width:92%;height:8px"></i><i class="bar soft" style="width:78%;height:8px"></i></div>"#,
        )),
        "deck-comparison" => slide(&format!(
            r#"<div class="col" style="gap:18px;padding:38px 48px">{t}
<div class="col" style="border:1px solid var(--ds-color-border,#e4e7ec);border-radius:12px;overflow:hidden;margin-top:8px">
{head}{row1}{row2}{row3}</div></div>"#,
            t = bar("44%", 20, "strong"),
            head = comparison_row(true),
            row1 = comparison_row(false),
            row2 = comparison_row(false),
            row3 = comparison_row(false),
        )),
        "deck-datastory" => slide(&format!(
            r#"<div class="col" style="gap:14px;padding:38px 48px">{t}
<div class="mutedcard" style="border-radius:12px;padding:20px;margin-top:6px">{chart}</div>
<div class="row" style="gap:10px;align-items:center;margin-top:4px">
<i class="chip" style="width:26px;height:14px;border-radius:99px;background:var(--ds-color-primary,#4f46e5)"></i>{insight}</div></div>"#,
            t = bar("38%", 20, "strong"),
            chart = chart_line(640, 200),
            insight = bar("58%", 11, "strong"),
        )),
        "dashboard-admin" => format!(
            r#"<div class="row" style="height:100%">
<div class="col" style="width:170px;gap:14px;border-right:1px solid var(--ds-color-border,#e4e7ec);padding:20px 16px;background:var(--ds-color-muted,#f5f6f8)">
<div class="row" style="gap:8px;align-items:center"><i class="chip" style="width:22px;height:22px;border-radius:7px;background:var(--ds-color-primary,#4f46e5)"></i>{logo}</div>
<div style="height:8px"></div>{navon}{navoff}{navoff}{navoff}{navoff}</div>
<div class="col" style="flex:1;gap:14px;padding:18px 22px;min-width:0">
<div class="row" style="gap:10px;align-items:center">{title}<span style="flex:1"></span>{filter}{filter}</div>
<div class="row" style="gap:12px">{k}{k}{k}{k}</div>
<div class="row" style="gap:12px">
<div class="card" style="flex:1.4;padding:14px">{line}</div>
<div class="card" style="flex:1;padding:14px">{bars}</div></div>
<div class="card col" style="overflow:hidden">{table}</div></div></div>"#,
            logo = bar("64px", 11, "strong"),
            navon = r#"<div class="row" style="gap:8px;align-items:center;background:color-mix(in srgb,var(--ds-color-primary,#4f46e5) 12%,transparent);border-radius:8px;padding:8px 10px"><i class="chip" style="width:12px;height:12px;border-radius:4px;background:var(--ds-color-primary,#4f46e5)"></i><i class="bar strong" style="width:60%;height:9px"></i></div>"#,
            navoff = r#"<div class="row" style="gap:8px;align-items:center;padding:8px 10px"><i class="chip" style="width:12px;height:12px;border-radius:4px;background:var(--ds-color-border,#dbe0e6)"></i><i class="bar soft" style="width:60%;height:9px"></i></div>"#,
            title = bar("140px", 15, "strong"),
            filter = ghost_btn(72),
            k = kpi_card(false),
            line = chart_line(360, 130),
            bars = chart_bars(250, 130),
            table = table_rows(3),
        ),
        "dashboard-analytics" => format!(
            r#"<div class="col" style="gap:14px;padding:20px 26px;height:100%">
<div class="row" style="gap:10px;align-items:center">{title}<span style="flex:1"></span>{f}{f}{f}</div>
<div class="row" style="gap:12px">{k}{k}{k}{k}</div>
<div class="card" style="padding:16px">{big}</div>
<div class="row" style="gap:12px">
<div class="card" style="flex:1;padding:13px">{s1}</div>
<div class="card" style="flex:1;padding:13px">{s2}</div></div></div>"#,
            title = bar("150px", 15, "strong"),
            f = ghost_btn(70),
            k = kpi_card(true),
            big = chart_line(800, 160),
            s1 = chart_bars(260, 90),
            s2 = chart_line(260, 90),
        ),
        "poster-social" => poster_frame(
            460,
            460,
            &format!(
                r#"<i class="chip" style="position:absolute;inset:0;background:linear-gradient(160deg,color-mix(in srgb,var(--ds-color-primary,#4f46e5) 80%,transparent),color-mix(in srgb,var(--ds-color-accent,var(--ds-color-primary,#4f46e5)) 36%,#000 6%))"></i>
<i class="chip" style="position:absolute;right:-40px;bottom:-40px;width:200px;height:200px;border-radius:99px;background:color-mix(in srgb,#fff 16%,transparent)"></i>
<div class="col" style="position:absolute;left:36px;top:44px;gap:14px">{t1}{t2}<div style="height:2px"></div>{sub}</div>
<div class="row" style="position:absolute;left:36px;bottom:30px;gap:8px;align-items:center">
<i class="chip" style="width:20px;height:20px;border-radius:6px;background:#fff;opacity:.92"></i>{brand}</div>"#,
                t1 = bar("240px", 26, "inv"),
                t2 = bar("170px", 26, "inv"),
                sub = bar("200px", 11, "inv"),
                brand = bar("70px", 10, "inv"),
            ),
        ),
        "poster-event" => poster_frame(
            420,
            525,
            &format!(
                r#"<i class="chip" style="position:absolute;inset:0;background:linear-gradient(200deg,color-mix(in srgb,var(--ds-color-fg,#111) 88%,transparent),color-mix(in srgb,var(--ds-color-primary,#4f46e5) 55%,#000 20%))"></i>
<i class="chip" style="position:absolute;left:-50px;top:90px;width:180px;height:180px;border-radius:99px;border:2px solid color-mix(in srgb,#fff 30%,transparent);background:transparent"></i>
<div class="col" style="position:absolute;left:32px;top:52px;gap:13px">{k}{t1}{t2}</div>
<div class="col" style="position:absolute;left:32px;bottom:86px;gap:9px">{m1}{m2}</div>
<div class="row" style="position:absolute;left:32px;right:32px;bottom:26px;align-items:end;justify-content:space-between">
{brand}<i class="chip" style="width:44px;height:44px;border-radius:8px;background:#fff;opacity:.9"></i></div>"#,
                k = bar("80px", 10, "pri"),
                t1 = bar("250px", 24, "inv"),
                t2 = bar("180px", 24, "inv"),
                m1 = bar("150px", 10, "inv"),
                m2 = bar("120px", 10, "inv"),
                brand = bar("70px", 10, "inv"),
            ),
        ),
        "document-spec" => doc_page(&format!(
            r#"{t}<div class="row" style="gap:10px">{m}{m}{m}</div>
<div class="mutedcard col" style="gap:9px;padding:14px 16px;margin-top:4px">{toc}{toc}{toc}</div>
<div style="height:4px"></div>{h}{p}{p}{p2}<div style="height:4px"></div>{h2}{p}{p2}"#,
            t = bar("62%", 20, "strong"),
            m = bar("70px", 9, "soft"),
            toc = r#"<div class="row" style="gap:8px;align-items:center"><i class="chip" style="width:6px;height:6px;border-radius:99px;background:var(--ds-color-primary,#4f46e5)"></i><i class="bar soft" style="width:44%;height:9px"></i></div>"#,
            h = bar("34%", 14, "strong"),
            h2 = bar("40%", 14, "strong"),
            p = bar("100%", 9, "soft"),
            p2 = bar("74%", 9, "soft"),
        )),
        "document-okr" => doc_page(&format!(
            r#"{t}<div class="row" style="gap:10px">{m}{m}</div>
{obj}{obj2}"#,
            t = bar("52%", 20, "strong"),
            m = bar("80px", 9, "soft"),
            obj = okr_card(
                "62%",
                &[
                    ("72%", 82, "#16a34a"),
                    ("58%", 55, "#d97706"),
                    ("66%", 30, "#dc2626")
                ]
            ),
            obj2 = okr_card("48%", &[("64%", 70, "#16a34a"), ("52%", 45, "#d97706")]),
        )),
        "document-runbook" => doc_page(&format!(
            r#"<div class="row" style="gap:10px;align-items:center">{t}<i class="chip" style="width:56px;height:18px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-danger,#dc2626) 18%,transparent)"></i></div>
<div class="row" style="gap:10px">{m}{m}</div>
{step1}{code}{step2}{step3}"#,
            t = bar("54%", 20, "strong"),
            m = bar("76px", 9, "soft"),
            step1 = runbook_step(1, false),
            step2 = runbook_step(2, true),
            step3 = runbook_step(3, false),
            code = r#"<div class="chip" style="height:52px;border-radius:10px;background:color-mix(in srgb,var(--ds-color-fg,#16181d) 88%,transparent);padding:12px 14px"><i class="bar" style="display:block;width:52%;height:8px;background:#fff;opacity:.55;border-radius:99px"></i><i class="bar" style="display:block;width:34%;height:8px;background:#fff;opacity:.35;border-radius:99px;margin-top:8px"></i></div>"#,
        )),
        "document-report" => doc_page(&format!(
            r#"{t}{sub}
<div class="row" style="gap:10px;margin-top:2px">{k}{k}{k}</div>
<div class="mutedcard" style="border-radius:10px;padding:14px;margin-top:4px">{chart}</div>
{p}{p2}"#,
            t = bar("58%", 20, "strong"),
            sub = bar("80%", 10, "soft"),
            k = kpi_card(true),
            chart = chart_line(480, 110),
            p = bar("100%", 9, "soft"),
            p2 = bar("70%", 9, "soft"),
        )),
        "document-onboarding" => doc_page(&format!(
            r#"{t}{sub}
<div class="row" style="gap:12px;margin-top:6px">{s1}{s2}{s3}</div>
<div style="height:2px"></div>{li}{li}{li}"#,
            t = bar("50%", 20, "strong"),
            sub = bar("64%", 10, "soft"),
            s1 = onboarding_stage("var(--ds-color-primary,#4f46e5)"),
            s2 = onboarding_stage(
                "color-mix(in srgb,var(--ds-color-primary,#4f46e5) 55%,transparent)"
            ),
            s3 = onboarding_stage(
                "color-mix(in srgb,var(--ds-color-primary,#4f46e5) 26%,transparent)"
            ),
            li = r#"<div class="row" style="gap:10px;align-items:center"><i class="chip" style="width:14px;height:14px;border-radius:4px;border:1.5px solid var(--ds-color-border,#d5dae0);background:transparent"></i><i class="bar soft" style="width:70%;height:9px"></i></div>"#,
        )),
        "document-rfc" => doc_page(&format!(
            r#"<div class="row" style="gap:10px;align-items:center">{t}<i class="chip" style="width:64px;height:18px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-success,#16a34a) 20%,transparent)"></i></div>
{p}{p2}
<div class="col" style="border:1px solid var(--ds-color-border,#e4e7ec);border-radius:10px;overflow:hidden;margin-top:4px">{head}{r1}{r2}</div>
{p3}{p4}"#,
            t = bar("50%", 20, "strong"),
            p = bar("100%", 9, "soft"),
            p2 = bar("82%", 9, "soft"),
            head = comparison_row(true),
            r1 = comparison_row(false),
            r2 = comparison_row(false),
            p3 = bar("96%", 9, "soft"),
            p4 = bar("64%", 9, "soft"),
        )),
        "email-marketing" => email_frame(&format!(
            r#"<i class="chip" style="height:150px;border-radius:0;background:linear-gradient(135deg,color-mix(in srgb,var(--ds-color-primary,#4f46e5) 70%,transparent),color-mix(in srgb,var(--ds-color-accent,var(--ds-color-primary,#4f46e5)) 30%,transparent))"></i>
<div class="col" style="align-items:center;gap:11px;padding:26px 40px 30px">
{t}{p}{p2}<div style="height:6px"></div>{b}
<div style="height:10px"></div><i style="display:block;width:100%;height:1px;background:var(--ds-color-border,#e4e7ec)"></i>
<div class="row" style="gap:8px;margin-top:4px">{f}{f}{f}</div></div>"#,
            t = bar("62%", 17, "strong"),
            p = bar("88%", 9, "soft"),
            p2 = bar("70%", 9, "soft"),
            b = btn(150),
            f = bar("54px", 8, "soft"),
        )),
        "email-transactional" => email_frame(&format!(
            r#"<div class="row" style="gap:9px;align-items:center;padding:16px 28px;border-bottom:1px solid var(--ds-color-border,#e4e7ec)">
<i class="chip" style="width:20px;height:20px;border-radius:6px;background:var(--ds-color-primary,#4f46e5)"></i>{logo}</div>
<div class="col" style="gap:12px;padding:24px 28px">
{t}{sub}
<div class="mutedcard col" style="gap:0;border-radius:10px;overflow:hidden;margin-top:4px">{kv}{kv}{kv}</div>
<div style="height:4px"></div>{b}
<div style="height:8px"></div>{foot}</div>"#,
            logo = bar("60px", 10, "strong"),
            t = bar("46%", 16, "strong"),
            sub = bar("72%", 9, "soft"),
            kv = r#"<div class="row" style="justify-content:space-between;padding:11px 14px;border-bottom:1px solid var(--ds-color-border,#e7eaee)"><i class="bar soft" style="width:26%;height:8px"></i><i class="bar strong" style="width:20%;height:8px"></i></div>"#,
            b = btn(130),
            foot = bar("50%", 8, "soft"),
        )),
        "motion-kinetic" => motion_stage(&format!(
            r#"<div class="col" style="align-items:center;justify-content:center;gap:18px;height:100%">
<i class="bar inv m1" style="width:300px;height:28px"></i>
<i class="bar m2" style="width:210px;height:28px;background:var(--ds-color-primary,#5b7cfa)"></i>
<i class="bar inv m3" style="width:140px;height:12px;opacity:.6"></i></div>"#
        )),
        "motion-reveal" => motion_stage(&format!(
            r#"<div class="col" style="align-items:center;justify-content:center;gap:22px;height:100%">
<i class="chip mz" style="width:150px;height:96px;border-radius:16px;background:linear-gradient(135deg,var(--ds-color-primary,#5b7cfa),color-mix(in srgb,var(--ds-color-primary,#5b7cfa) 40%,#fff 12%))"></i>
<div class="col" style="align-items:center;gap:10px">
<i class="bar inv m2" style="width:200px;height:16px"></i>
<i class="bar inv m3" style="width:130px;height:10px;opacity:.55"></i></div></div>"#
        )),
        _ => return None,
    };
    Some(html)
}

// ── 复合小件 ───────────────────────────────────────────────────────

/// 定价卡（highlight = 推荐档主色描边 + 按钮实色）。
fn pricing_card(highlight: bool) -> String {
    let (border, button) = if highlight {
        (
            "border:2px solid var(--ds-color-primary,#4f46e5);",
            r#"<i class="btn" style="width:100%;height:32px"></i>"#,
        )
    } else {
        (
            "",
            r#"<i class="ghostbtn" style="width:100%;height:32px"></i>"#,
        )
    };
    format!(
        r#"<div class="card col" style="flex:1;gap:10px;padding:18px;{border}">
{name}{price}
<div class="col" style="gap:8px;margin:6px 0 4px">{li}{li}{li}</div>{button}</div>"#,
        name = bar("40%", 10, "soft"),
        price = bar("54%", 18, "strong"),
        li = r#"<div class="row" style="gap:8px;align-items:center"><i class="chip" style="width:10px;height:10px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-success,#16a34a) 32%,transparent)"></i><i class="bar soft" style="width:74%;height:8px"></i></div>"#,
    )
}

/// 时间线节点（left%，done = 主色实心）。
fn timeline_dot(left_pct: u32, done: bool) -> String {
    let fill = if done {
        "var(--ds-color-primary,#4f46e5)"
    } else {
        "var(--ds-color-border,#d5dae0)"
    };
    format!(
        r#"<i class="chip" style="position:absolute;left:{left_pct}%;top:-5px;width:16px;height:16px;border-radius:99px;border:3px solid var(--ds-color-bg,#fff);background:{fill}"></i>"#
    )
}

/// 对比表行（head = muted 底；数据行末两列为勾/叉色点）。
fn comparison_row(head: bool) -> String {
    if head {
        return format!(
            r#"<div class="row" style="gap:16px;padding:12px 16px;background:var(--ds-color-muted,#f1f3f6)">{}{}{}</div>"#,
            bar("30%", 9, "strong"),
            bar("18%", 9, "strong"),
            bar("18%", 9, "strong"),
        );
    }
    format!(
        r#"<div class="row" style="gap:16px;padding:12px 16px;align-items:center;border-top:1px solid var(--ds-color-border,#e7eaee)">{}
<div class="row" style="width:18%;justify-content:flex-start"><i class="chip" style="width:14px;height:14px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-success,#16a34a) 36%,transparent)"></i></div>
<div class="row" style="width:18%"><i class="chip" style="width:14px;height:14px;border-radius:99px;background:color-mix(in srgb,var(--ds-color-danger,#dc2626) 26%,transparent)"></i></div></div>"#,
        bar("30%", 9, "soft"),
    )
}

/// OKR objective 卡：目标句 + 若干 KR 进度条（各带完成度与信心色）。
fn okr_card(title_w: &str, krs: &[(&str, u32, &str)]) -> String {
    let mut kr_html = String::new();
    for (w, pct, color) in krs {
        kr_html.push_str(&format!(
            r#"<div class="col" style="gap:6px"><div class="row" style="justify-content:space-between;align-items:center"><i class="bar soft" style="width:{w};height:8px"></i><i class="chip" style="width:22px;height:8px;border-radius:99px;background:color-mix(in srgb,{color} 30%,transparent)"></i></div>
<div style="height:6px;border-radius:99px;background:var(--ds-color-muted,#eef0f3)"><i class="chip" style="display:block;width:{pct}%;height:6px;border-radius:99px;background:{color}"></i></div></div>"#
        ));
    }
    format!(
        r#"<div class="card col" style="gap:12px;padding:16px 18px">{}{kr_html}</div>"#,
        bar(title_w, 12, "strong"),
    )
}

/// Runbook 编号步骤（danger = 高危高亮）。
fn runbook_step(n: u32, danger: bool) -> String {
    let ring = if danger {
        "background:color-mix(in srgb,var(--ds-color-danger,#dc2626) 16%,transparent);color-scheme:light"
    } else {
        "background:var(--ds-color-muted,#eef0f3)"
    };
    let _ = n;
    format!(
        r#"<div class="row" style="gap:12px;align-items:flex-start">
<i class="chip" style="width:22px;height:22px;border-radius:99px;{ring}"></i>
<div class="col" style="flex:1;gap:7px;padding-top:2px"><i class="bar strong" style="width:56%;height:10px"></i><i class="bar soft" style="width:84%;height:8px"></i></div></div>"#
    )
}

/// 入职阶段卡（30/60/90 色带）。
fn onboarding_stage(band: &str) -> String {
    format!(
        r#"<div class="card col" style="flex:1;gap:9px;padding:0 0 12px;overflow:hidden">
<i class="chip" style="height:8px;border-radius:0;background:{band}"></i>
<div class="col" style="gap:8px;padding:6px 14px 0"><i class="bar strong" style="width:44%;height:11px"></i><i class="bar soft" style="width:86%;height:8px"></i><i class="bar soft" style="width:70%;height:8px"></i></div></div>"#
    )
}

/// 动效舞台：深底 16:9 + 循环入场动画（骨架真的会动）。
fn motion_stage(inner: &str) -> String {
    format!(
        r#"<div class="row" style="justify-content:center;align-items:center;height:100%;background:var(--ds-color-muted,#eef0f3)">
<div style="width:780px;height:440px;border-radius:14px;overflow:hidden;position:relative;background:color-mix(in srgb,var(--ds-color-fg,#0d0f14) 92%,#000)">{inner}</div></div>"#
    )
}

// ── 组装 ───────────────────────────────────────────────────────────

/// 生成 recipe 骨架 demo 自包含 HTML；未知 recipe 返回 None。
pub fn build_recipe_demo_html(
    recipe_id: &str,
    tokens: &BTreeMap<String, String>,
) -> Option<String> {
    let body = demo_body(recipe_id)?;
    let token_vec: Vec<(String, String)> =
        tokens.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let root = tokens_root_css(&token_vec);
    Some(format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<style>
{root}
:root{{color-scheme:light}}
*,*::before,*::after{{box-sizing:border-box;margin:0;padding:0}}
html,body{{width:{w}px;height:{h}px;overflow:hidden}}
body{{background:var(--ds-color-bg,#fff);font-family:var(--ds-font-sans,system-ui,sans-serif)}}
i{{display:block}}
.row{{display:flex}}
.col{{display:flex;flex-direction:column}}
.bar{{border-radius:99px;background:var(--ds-color-fg,#16181d)}}
.bar.soft{{opacity:.26}}
.bar.strong{{opacity:.8}}
.bar.inv{{background:#fff;opacity:.92}}
.bar.pri{{background:var(--ds-color-primary,#4f46e5);opacity:1}}
.chip{{border-radius:8px}}
.btn{{border-radius:9px;background:var(--ds-color-primary,#4f46e5)}}
.ghostbtn{{border-radius:9px;border:1.5px solid var(--ds-color-border,#dfe3e8);background:transparent}}
.card{{border-radius:12px;border:1px solid var(--ds-color-border,#e4e7ec);background:var(--ds-color-bg,#fff);box-shadow:0 1px 2px rgba(15,23,42,.05)}}
.mutedcard{{background:var(--ds-color-muted,#f1f3f6)}}
@keyframes ds-rise{{0%{{opacity:0;transform:translateY(26px)}}14%{{opacity:1;transform:none}}72%{{opacity:1;transform:none}}86%{{opacity:0;transform:translateY(-10px)}}100%{{opacity:0;transform:translateY(-10px)}}}}
@keyframes ds-zoom{{0%{{opacity:0;transform:scale(.7)}}16%{{opacity:1;transform:scale(1)}}72%{{opacity:1;transform:scale(1)}}86%{{opacity:0;transform:scale(1.04)}}100%{{opacity:0}}}}
.m1{{animation:ds-rise 5.5s ease .0s infinite}}
.m2{{animation:ds-rise 5.5s ease .35s infinite;opacity:0}}
.m3{{animation:ds-rise 5.5s ease .7s infinite;opacity:0}}
.mz{{animation:ds-zoom 5.5s cubic-bezier(.22,1,.36,1) infinite}}
</style></head><body>
{body}
</body></html>"#,
        w = DEMO_CANVAS_W,
        h = DEMO_CANVAS_H,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个内置 recipe 都必须有骨架 demo（新增 recipe 忘配 demo 时此测试失败）。
    #[test]
    fn every_builtin_recipe_has_demo() {
        for r in super::super::recipe::builtin_recipes() {
            assert!(
                demo_body(&r.id).is_some(),
                "recipe `{}` 缺骨架 demo（recipe_demo.rs::demo_body 补分支）",
                r.id
            );
        }
    }

    #[test]
    fn unknown_recipe_returns_none() {
        assert!(build_recipe_demo_html("no-such", &BTreeMap::new()).is_none());
    }

    /// tokens 注入进 `:root` 且骨架变量引用存在。
    #[test]
    fn tokens_are_injected() {
        let mut tokens = BTreeMap::new();
        tokens.insert("--ds-color-primary".to_string(), "#d97757".to_string());
        let html = build_recipe_demo_html("web-landing", &tokens).unwrap();
        assert!(html.contains("--ds-color-primary:#d97757"));
        assert!(html.contains("var(--ds-color-primary"));
    }
}

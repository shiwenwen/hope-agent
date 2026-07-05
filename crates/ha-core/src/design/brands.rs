//! 品牌风格参考设计系统（数据层）。
//!
//! 每个 [`BrandSeed`] 只声明品牌签名色 / 字体 / 圆角 / 字号密度 / 气质，运行期由
//! [`super::system::expand`] 展开为完整 25 token 契约的设计系统，渲染时自动附免责声明。
//!
//! 全部为对各品牌**公开视觉语言的独立再诠释**，仅供设计参考；非官方、与各品牌及其
//! 权利人无任何隶属 / 赞助 / 授权关系，相关名称与商标归各自所有者所有。

use super::system::{BrandSeed, Radius, Scale};

/// 构造一个品牌种子（位置参数顺序即字段顺序，便于批量维护）。
#[allow(clippy::too_many_arguments)]
fn b(
    id: &'static str,
    name: &'static str,
    brand_ref: &'static str,
    summary: &'static str,
    bg: &'static str,
    fg: &'static str,
    primary: &'static str,
    accent: &'static str,
    muted: &'static str,
    border: &'static str,
    font: &'static str,
    display_font: &'static str,
    radius: Radius,
    scale: Scale,
    doc: &'static str,
) -> BrandSeed {
    BrandSeed {
        id,
        name,
        brand_ref,
        summary,
        // 分节统一由 `cat(..)` 赋值。
        category: "",
        bg,
        fg,
        primary,
        accent,
        muted,
        border,
        font,
        display_font,
        radius,
        scale,
        doc,
    }
}

/// 给一批种子统一打上分组类目。
fn cat(category: &'static str, mut list: Vec<BrandSeed>) -> Vec<BrandSeed> {
    for s in &mut list {
        s.category = category;
    }
    list
}

/// 全部品牌风格参考种子。新增品牌在对应分节里追加一行即可。
pub(super) fn seeds() -> Vec<BrandSeed> {
    use Radius::*;
    use Scale::*;
    let mut v: Vec<BrandSeed> = Vec::new();
    v.extend(cat(
        "开发者工具",
        vec![
        // ── 开发者工具 / 基础设施 ──────────────────────────────
        b("brand-linear", "Linear", "Linear", "克制精密的深色开发者美学", "#08090A", "#F7F8F8", "#5E6AD2", "", "#1C1D22", "#23252A", "'Inter Variable',Inter,system-ui,'PingFang SC',sans-serif", "", Small, Compact, "近黑背景配单一靛蓝紫强调色，细线分层、留白克制，字重变化传递层级。避免高饱和撞色与浓重投影，禁止用于营造喧闹促销感。"),
        b("brand-vercel", "Vercel", "Vercel Inc.", "极简黑白双色的工程师美学", "#000000", "#FFFFFF", "#000000", "", "#111111", "#262626", "'Geist Sans',Geist,-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,'PingFang SC',sans-serif", "", Sharp, Normal, "纯黑白双色、无多余点缀，几何感排版与充足留白营造工程精密感。忌加入任何非必要的彩色点缀或渐变装饰。"),
        b("brand-github", "GitHub", "GitHub, Inc.", "亲和白底配蓝绿双色的协作感", "#FFFFFF", "#1F2328", "#0969DA", "#1F883D", "#F6F8FA", "#D0D7DE", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'PingFang SC',sans-serif", "", Small, Compact, "浅灰白底、蓝色交互与绿色成功态构成协作氛围，边框分隔信息密集的列表。避免大面积纯黑背景与过度圆润的按钮。"),
        b("brand-gitlab", "GitLab", "GitLab Inc.", "橙紫双色的开发流水线活力感", "#FFFFFF", "#1F1E24", "#FC6D26", "#6E49CB", "#F5F5F5", "#DBDBDB", "'GitLab Sans',-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC',sans-serif", "", Small, Compact, "橙色主色搭配紫色点缀，浅底深字承载密集的流水线与合并请求信息。避免柔和低对比配色削弱状态可读性。"),
        b("brand-netlify", "Netlify", "Netlify, Inc.", "深青绿背景的友好云端质感", "#0E1E25", "#FFFFFF", "#00C7B7", "", "#16292F", "#22383F", "'Inter',system-ui,-apple-system,'PingFang SC',sans-serif", "", Rounded, Normal, "深青绿背景配明快青色强调，圆润按钮与柔和卡片传递亲和的部署体验。避免生硬直角与冷峻纯灰配色。"),
        b("brand-railway", "Railway", "Railway Corp.", "极暗背景配单一紫色的极客感", "#0B0B0D", "#F2F0F5", "#9D5CFF", "", "#17161C", "#262530", "'Inter',system-ui,-apple-system,'PingFang SC',sans-serif", "", Medium, Compact, "近黑背景配单一紫色点缀，克制的卡片与等距留白呈现基础设施部署的精密感。避免多色混搭与花哨渐变喧宾夺主。"),
        b("brand-supabase", "Supabase", "Supabase Inc.", "深灰背景配祖母绿的开源活力", "#171717", "#EDEDED", "#3ECF8E", "", "#242424", "#2E2E2E", "'Inter',system-ui,-apple-system,'PingFang SC',sans-serif", "", Medium, Compact, "深灰背景衬托高饱和祖母绿主色，等宽字点缀数据表格与代码。避免使用暖色系点缀冲淡绿色的辨识度。"),
        b("brand-render", "Render", "Render (Render Services, Inc.)", "清爽白底配绿紫双色的现代云感", "#FFFFFF", "#1A1A1A", "#00DB7C", "#8A05FF", "#F6F6F6", "#E3E3E3", "'Inter',system-ui,-apple-system,'PingFang SC',sans-serif", "", Medium, Normal, "明快白底以荧光绿为主、电紫为点缀，简洁卡片突出部署状态。避免灰暗低对比背景削弱双色反差。"),
        b("brand-cloudflare", "Cloudflare", "Cloudflare, Inc.", "白底醒目橙色的安全网络感", "#FFFFFF", "#0D1622", "#F38020", "", "#F6F8FA", "#DDE3EA", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'PingFang SC',sans-serif", "", Small, Compact, "白底深灰文字配标志性橙色点缀，克制表格化布局呈现网络与安全配置。避免使用高饱和多色图表喧宾夺主。"),
        b("brand-docker", "Docker", "Docker, Inc.", "白底鲸鱼蓝的容器化清爽感", "#FFFFFF", "#17202A", "#2496ED", "", "#F5F9FC", "#D9E4EC", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'PingFang SC',sans-serif", "", Small, Normal, "白底配单一鲸鱼蓝强调色，简洁图标化语言呈现容器编排概念。避免多彩渐变与厚重阴影破坏工具感。"),
        b("brand-datadog", "Datadog", "Datadog, Inc.", "白底紫色的监控数据精密感", "#FFFFFF", "#13141A", "#632CA6", "", "#F7F5FA", "#E4DEEE", "'Proxima Nova',-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC',sans-serif", "", Small, Compact, "白底以深紫为签名色，密集图表与表格承载可观测性数据。避免用高饱和撞色图例削弱紫色的品牌辨识度。"),
        b("brand-sentry", "Sentry", "Functional Software, Inc. (Sentry)", "深紫背景配灼热橙红的错误警示感", "#362D59", "#EDE9F5", "#FB4226", "", "#443868", "#4F4278", "Rubik,-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC',sans-serif", "", Medium, Compact, "深紫背景配灼热橙红点缀，警示色专用于错误与告警语义。避免把橙红用于常规装饰、稀释其「出错」信号强度。"),
        b("brand-grafana", "Grafana", "Grafana Labs", "深色底配橙色的可观测仪表盘感", "#111217", "#D8D9DA", "#F46800", "", "#1A1B20", "#2C3235", "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,'PingFang SC',sans-serif", "", Small, Compact, "深色底承载密集图表面板，橙色仅用于关键指标与告警高亮。避免大面积高饱和橙色导致视觉疲劳。"),
        b("brand-postman", "Postman", "Postman, Inc.", "白底醒目橙色的接口调试活力", "#FFFFFF", "#232323", "#FF6C37", "", "#F7F7F7", "#E0E0E0", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'PingFang SC',sans-serif", "", Medium, Normal, "白底配高饱和橙色主色，圆润按钮与清晰分区呈现请求构建流程。避免暗色主题下仍大面积铺橙造成刺眼。"),
        b("brand-replit", "Replit", "Replit, Inc.", "深色底配橙红的社区编程活力", "#0E1525", "#F5F9FC", "#F26207", "", "#172032", "#232C3D", "'Söhne',system-ui,-apple-system,'PingFang SC',sans-serif", "", Rounded, Normal, "深蓝黑底配活泼橙红与大量圆角，营造友好易上手的编程社区氛围。避免尖锐直角与冷峻纯灰破坏亲和感。"),
        b("brand-raycast", "Raycast", "Raycast Technologies Ltd.", "深色底配珊瑚红的原生工具感", "#0D0D0D", "#F2F2F2", "#FF6363", "", "#1C1C1E", "#2C2C2E", "-apple-system,BlinkMacSystemFont,'SF Pro Text','PingFang SC',sans-serif", "", Medium, Compact, "深色磨砂底配珊瑚红点缀，列表化命令面板呼应桌面原生质感。避免高对比撞色与厚重描边破坏轻盈感。"),
        b("brand-warp", "Warp", "Warp.dev, Inc.", "暗色终端配蓝紫渐变的未来感", "#0B0B0F", "#F5F5F7", "#4F7DF3", "", "#17171C", "#2A2A31", "'Berkeley Mono','JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,'PingFang SC',monospace", "", Rounded, Compact, "近黑终端底配蓝紫强调与等宽字体，块状命令区圆角柔化传统终端的生硬感。避免使用衬线字体与低对比灰字。"),
        b("brand-hashicorp", "HashiCorp", "HashiCorp, Inc.", "极简黑白的基础设施工程感", "#FFFFFF", "#000000", "#000000", "", "#F5F5F5", "#E1E1E1", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC',sans-serif", "", Sharp, Normal, "纯黑白双色、几何六边形符号语言，克制排版突出基础设施的严谨可靠。避免引入单一产品线专属色代表整体品牌。"),
        b("brand-fly-io", "Fly.io", "Fly.io, Inc.", "深紫夜空色的边缘部署未来感", "#170F27", "#F5F1FF", "#8438FA", "", "#211530", "#2E1F45", "'Inter',system-ui,-apple-system,'PingFang SC',sans-serif", "", Medium, Compact, "深紫近黑背景配明亮紫色强调，简洁排版呼应全球边缘节点的科技感。避免高明度背景冲淡夜空紫的氛围。"),
        ],
    ));
    v.extend(cat(
        "AI 产品",
        vec![
        // ── AI 产品 ─────────────────────────────────────────────
        b("brand-openai", "OpenAI", "OpenAI", "克制黑白灰基调点缀标志青绿", "#ffffff", "#0d0d0d", "#10a37f", "", "#f7f7f8", "#e5e5e7", "'Söhne',ui-sans-serif,-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "以极简黑白灰为基调，仅用标志性青绿色点缀关键交互元素，克制而聚焦。忌大面积滥用彩色渐变或多色堆叠，破坏其克制技术感。"),
        b("brand-anthropic", "Anthropic", "Anthropic", "暖米白纸感背景配陶土橙", "#faf9f5", "#141413", "#d97757", "#6a9bcc", "#e8e6dc", "#e8e6dc", "'Styrene A','Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "'Tiempos Headline',Georgia,'Times New Roman',serif,'PingFang SC'", Rounded, Normal, "暖调米白纸张质感背景搭配陶土橙主色与衬线标题，传达人文温度与克制的学术气质。忌使用冷白背景或高饱和荧光色，破坏纸感温度。"),
        b("brand-perplexity", "Perplexity", "Perplexity", "米纸色背景配靛青绿编辑感", "#fbfaf4", "#202020", "#20808d", "", "#f0eee2", "#e5e1d3", "'FK Grotesk Neue','Inter',-apple-system,sans-serif,'PingFang SC'", "'Tiempos Text',Georgia,serif,'PingFang SC'", Medium, Normal, "米色纸张质感背景搭配靛青绿强调色与衬线正文，营造类似百科全书的可信编辑感。忌用鲜蓝或紫色替代招牌青绿，丢失品牌辨识度。"),
        b("brand-hugging-face", "Hugging Face", "Hugging Face", "明黄配黑白的亲切社区活泼感", "#ffffff", "#232323", "#ffd21e", "", "#fff6d9", "#f0e6c0", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif,'PingFang SC'", "'Poppins','Inter',sans-serif,'PingFang SC'", Rounded, Compact, "明黄主色搭配黑白基底与圆润造型，营造开源社区亲切活泼的气质。忌使用严肃深色系或尖角设计，削弱平易近人感。"),
        b("brand-midjourney", "Midjourney", "Midjourney", "纯黑画廊感让图像成为唯一色彩", "#000000", "#ececec", "#ececec", "", "#141414", "#242424", "'Neue Haas Grotesk','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "'GT Sectra',Georgia,'Times New Roman',serif,'PingFang SC'", Small, Display, "纯黑白极简画廊式界面，克制到近乎无色，只为衬托生成图像本身的绚丽色彩。忌引入品牌彩色元素喧宾夺主，破坏画廊留白感。"),
        b("brand-runway", "Runway", "Runway", "电影感暗黑极简近乎无色", "#0a0a0a", "#fafafa", "#fafafa", "", "#1a1a1a", "#2a2a2a", "'Suisse International','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Display, "深黑影棚质感搭配极简无衬线字体，呈现专业影视工具的克制氛围。忌使用鲜艳饱和色系，削弱专业严肃调性。"),
        b("brand-cohere", "Cohere", "Cohere", "深灰配珊瑚橙理性带人性温度", "#fafafa", "#212121", "#ff7759", "", "#f0f0f0", "#e2e2e2", "'Söhne','Inter',-apple-system,sans-serif,'PingFang SC'", "", Medium, Normal, "深灰基底搭配珊瑚橙强调色，在理性技术感中注入自然温度。忌大面积使用冷调蓝紫，削弱其温暖人性化定位。"),
        b("brand-mistral-ai", "Mistral AI", "Mistral AI", "像素块国际橙棱角分明工业感", "#ffffff", "#17171a", "#fa520f", "#ffc300", "#f5f5f5", "#e8e8e8", "'Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "", Sharp, Normal, "像素模块化的国际橙主色搭配黑白基底与直角造型，呈现锐利工业气质。忌使用圆角或柔化渐变，破坏像素识别系统。"),
        b("brand-stripe", "Stripe", "Stripe", "靛蓝紫配深蓝的文档级精致感", "#ffffff", "#0a2540", "#635bff", "#00d4ff", "#f6f9fc", "#e3e8ee", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif,'PingFang SC'", "", Medium, Compact, "标志性靛蓝紫主色搭配深海军蓝文字与柔和渐变光泽，呈现开发者优先的精致技术感。忌使用高饱和暖色，破坏冷静专业的靛蓝识别度。"),
        b("brand-paypal", "PayPal", "PayPal", "经典深蓝配亮蓝的信赖支付感", "#ffffff", "#001c64", "#003087", "#0070e0", "#f5f7fa", "#e5e5e5", "'Helvetica Neue',Arial,-apple-system,sans-serif,'PingFang SC'", "", Pill, Normal, "深蓝与亮蓝的双色体系搭配胶囊形按钮，传达历史悠久、值得信赖的支付品牌感。忌引入过多辅助色，削弱单一蓝色体系的信任识别。"),
        b("brand-square", "Square", "Square", "纯黑白极简拒绝色彩的克制美学", "#ffffff", "#000000", "#000000", "", "#f7f7f7", "#e5e5e5", "'Akkurat','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "纯黑白双色的极简系统，刻意拒绝品牌色以彰显设计克制感与专业质感。忌添加任何强调色，这正是其品牌识别的核心禁忌。"),
        b("brand-robinhood", "Robinhood", "Robinhood", "荧光绿配黑白的年轻投资活力", "#000000", "#ffffff", "#00c805", "", "#141414", "#262626", "'Circular','Inter',-apple-system,sans-serif,'PingFang SC'", "", Rounded, Normal, "荧光绿主色搭配纯黑背景与白色文字，传达面向年轻世代的投资活力与颠覆感。忌使用传统金融的深蓝金色，丢失颠覆性年轻定位。"),
        b("brand-coinbase", "Coinbase", "Coinbase", "标志性钴蓝简洁可信加密感", "#ffffff", "#0a0b0d", "#0052ff", "", "#f5f7fa", "#e5e8eb", "'Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "", Medium, Normal, "单一钴蓝主色搭配大量留白与简洁几何图形，传达值得信赖的加密货币基础设施感。忌使用多彩渐变或荧光色，削弱稳健可信形象。"),
        b("brand-revolut", "Revolut", "Revolut", "深黑背景配宝蓝的高端极客感", "#191c1f", "#ffffff", "#4f55f1", "", "#24272b", "#2e3236", "'Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "", Rounded, Normal, "深色背景搭配宝蓝色强调与金属质感卡片，传达高端全球化的金融极客气质。忌使用暖色或柔和粉彩，削弱冷峻科技金融感。"),
        b("brand-wise", "Wise", "Wise", "荧光绿色块配森林绿的透明感", "#9fe870", "#163300", "#9fe870", "", "#c5f2a0", "#7acb4a", "'Wise Sans','Inter',-apple-system,sans-serif,'PingFang SC'", "", Rounded, Normal, "大面积荧光绿色块搭配深森林绿文字，传达「无隐藏费用」的坦诚透明气质。忌使用传统银行深蓝或金色，破坏反传统的透明定位。"),
        b("brand-cash-app", "Cash App", "Cash App", "纯黑配鲜绿的街头潮流支付态度", "#000000", "#ffffff", "#00d632", "", "#141414", "#262626", "'Cash Sans','Nunito',-apple-system,sans-serif,'PingFang SC'", "", Rounded, Normal, "纯黑背景搭配招牌鲜绿色与粗体圆角图形，传达街头潮流与个性化支付态度。忌使用白底或柔和色调，丢失黑绿撞色的强辨识度。"),
        b("brand-klarna", "Klarna", "Klarna", "标志性粉配黑的俏皮购物感", "#fff8f4", "#0a0a0a", "#ffb3c7", "", "#ffe3ea", "#f5d6df", "'Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "", Pill, Normal, "米白背景搭配招牌粉色块与纯黑文字，呈现更显质感的俏皮自信购物氛围。忌用蓝色或绿色替代粉色，粉色是不可妥协的核心资产。"),
        b("brand-plaid", "Plaid", "Plaid", "黑白极简配全息紫的基础设施感", "#ffffff", "#0a0a0a", "#0a0a0a", "#5b4cf5", "#f5f5f5", "#e5e5e5", "'Inter',-apple-system,BlinkMacSystemFont,sans-serif,'PingFang SC'", "", Small, Compact, "黑白极简基调点缀全息渐变紫色，呼应纸币防伪工艺的金融基础设施质感。忌大面积使用全息强调色，应作克制点缀而非主导。"),
        b("brand-venmo", "Venmo", "Venmo", "明快蓝配白的社交化转账感", "#ffffff", "#082c3e", "#008cff", "", "#eaf5ff", "#dceefc", "'Graphik','Inter',-apple-system,sans-serif,'PingFang SC'", "", Pill, Normal, "明快蓝色搭配大量留白与胶囊按钮，传达年轻人社交化转账的轻松随性感。忌使用严肃深色调，破坏社交支付的轻松属性。"),
        b("brand-monzo", "Monzo", "Monzo", "热珊瑚色配黑白的亲切银行感", "#ffffff", "#0a0a0a", "#ff4d56", "", "#ffe8e9", "#fdd3d6", "'Circular','Inter',-apple-system,sans-serif,'PingFang SC'", "", Rounded, Normal, "标志性热珊瑚色搭配简洁黑白与圆角卡片，传达亲切易懂的数字银行体验。忌使用传统银行深蓝或墨绿，削弱反传统的亲和力。"),
        ],
    ));
    v.extend(cat(
        "SaaS / 生产力",
        vec![
        // ── SaaS / 生产力 ───────────────────────────────────────
        b("brand-notion", "Notion", "Notion", "黑白极简克制留白的文档美学", "#ffffff", "#37352f", "#000000", "#2383e2", "#f7f6f3", "#e9e9e7", "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,'Apple Color Emoji',Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "黑白灰为骨架，纤细分割线与大量留白营造纸感克制的文档气质，蓝色仅作链接与强调点缀。忌堆砌鲜艳色块或粗重投影破坏素净留白感。"),
        b("brand-figma", "Figma", "Figma", "扁平多彩几何画布蓝紫点缀协作", "#ffffff", "#0d0d0d", "#0d99ff", "#a259ff", "#f5f5f5", "#e6e6e6", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "无限画布配扁平几何色块，蓝紫双色作交互强调，整体克制留白突出内容本身。忌给工具面板加拟物厚重阴影，破坏轻盈的画布感。"),
        b("brand-slack", "Slack", "Slack", "茄紫底色配四色标志的协作感", "#ffffff", "#1d1c1d", "#4a154b", "#36c5f0", "#f8f8f8", "#dddddd", "'Lato',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "茄紫侧边栏搭配四色标志与圆润气泡，传递轻松友好的团队协作氛围。忌整站大面积铺满茄紫深色，导致长时间阅读对比过重。"),
        b("brand-asana", "Asana", "Asana", "珊瑚粉暖色调圆点插画的轻快感", "#ffffff", "#1e1f21", "#f06a6a", "", "#f6f6f7", "#e8e8e9", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "珊瑚粉为核心的温暖亲和色调，圆角卡片与圆点插画传递轻快的任务协作感。忌用冷硬直角与高饱和撞色削弱其亲和力。"),
        b("brand-trello", "Trello", "Trello", "看板蓝配彩色标签的轻盈协作", "#ffffff", "#172b4d", "#0079bf", "#61bd4f", "#f4f5f7", "#dfe1e6", "'Charlie Text',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "'Charlie Display','Charlie Text',sans-serif,'PingFang SC'", Rounded, Normal, "看板蓝为底配彩色标签点缀，圆角卡片营造轻盈直观的拖拽协作感。忌卡片投影过重显得笨拙，偏离看板一目了然的轻量气质。"),
        b("brand-monday", "monday.com", "monday.com", "多彩状态色块的高能生产力视觉", "#ffffff", "#323338", "#ff3d57", "#ffcb00", "#f5f6f8", "#e6e9ef", "'Poppins','Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "高饱和状态色块铺满表格营造活力看板感，圆角与鲜明色彩强化可视化进度。忌状态色饱和度不足，丢失一眼识别的直觉性。"),
        b("brand-clickup", "ClickUp", "ClickUp", "高对比紫配蓝的全能生产力感", "#ffffff", "#292d34", "#7b68ee", "#0091ff", "#f8f8fc", "#e5e4f5", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "紫蓝渐变的高辨识主视觉搭配渐变插画，传递全能生产力工具的张扬感。忌功能堆砌让色彩噪音压过内容层级。"),
        b("brand-airtable", "Airtable", "Airtable", "蓝色主调配彩色积木图标表格感", "#ffffff", "#1d1f25", "#2d7ff9", "#f82b60", "#f7f8fa", "#e3e6eb", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Small, Normal, "电子表格式网格排布配四色积木图标点缀，蓝色为主强调理性可靠。忌网格线过重或色块过多让数据表失去清爽的可扫描性。"),
        b("brand-coda", "Coda", "Coda", "珊瑚橙手绘拼贴的可组合文档感", "#ffffff", "#101010", "#ee5a29", "", "#f5ede6", "#eae3dc", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "珊瑚橙搭配暖米白背景与手绘感图形拼贴，传递文档即应用的可组合性。忌把多彩图形色系直接套进正文排版造成杂乱。"),
        b("brand-miro", "Miro", "Miro", "亮黄品牌色配深靛蓝的白板感", "#ffffff", "#050038", "#ffd02f", "#4262ff", "#fafafa", "#e8e8f0", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "亮黄品牌色配深靛蓝文字与无限白板留白，营造开放的头脑风暴气质。忌大面积高饱和黄做正文背景导致长时间浏览刺眼。"),
        b("brand-loom", "Loom", "Loom", "靛紫渐变配珊瑚粉的柔和视频感", "#ffffff", "#1b1b1f", "#625df5", "#fa5f84", "#f5f4ff", "#e3e1fa", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Pill, Normal, "靛紫渐变搭配珊瑚粉点缀，圆形镜头图标与胶囊按钮传递轻松友好的异步视频感。忌用方正硬朗的直角元素削弱柔和亲和的调性。"),
        b("brand-calendly", "Calendly", "Calendly", "天蓝配深藏青清爽高效日程感", "#ffffff", "#0a2540", "#006bff", "", "#f2f7ff", "#d9e6f7", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "明亮天蓝色搭配深藏青文字，大量白色留白营造高效清爽的日程预约感。忌引入过多杂色标签打乱日历视图的秩序感。"),
        b("brand-zapier", "Zapier", "Zapier", "标志性橙色配深紫黑的自动化感", "#ffffff", "#201436", "#ff4a00", "", "#f8f5f2", "#ece7e3", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "高辨识度的橙色核心色搭配深紫黑文字，扁平化连接节点插画传达自动化流程感。忌橙色与其他高饱和色大面积并置造成刺眼撞色。"),
        b("brand-intercom", "Intercom", "Intercom", "靛蓝对话气泡配黑白的高对比感", "#ffffff", "#1c1e21", "#3e2bea", "#59a96e", "#f6f5ff", "#e4e1fa", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "靛蓝对话气泡配黑白基调与大胆排版，体现对话式客户支持的直接感。忌把标志性对话符号缩得过小，削弱品牌辨识度。"),
        b("brand-dropbox", "Dropbox", "Dropbox", "钴蓝配暖米白的手绘现代科技感", "#ffffff", "#1e1919", "#0061ff", "", "#f7f5f2", "#e5e2dd", "'Sharp Grotesk',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "明快钴蓝搭配温暖米白背景与手绘感插画，几何粗体字营造友好现代的科技感。忌把背景换成冷灰调，丢失标志性的暖调纸感。"),
        b("brand-zoom", "Zoom", "Zoom", "克制蓝白功能优先的通话工具感", "#ffffff", "#212121", "#2d8cff", "", "#f4f8ff", "#dde6f0", "'Lato',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "", Small, Compact, "简洁功能优先的蓝白配色，界面元素紧凑克制服务于视频通话本身。忌增加多余装饰色块分散通话画面的视觉焦点。"),
        b("brand-confluence", "Confluence", "Confluence", "蔚蓝配浅灰面板的层级协作感", "#ffffff", "#172b4d", "#2684ff", "#00b8d9", "#f4f5f7", "#dfe1e6", "'Charlie Text',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "'Charlie Display','Charlie Text',sans-serif,'PingFang SC'", Small, Normal, "蔚蓝色配浅灰面板的企业协作气质，页面结构层级分明强调长文档可读性。忌堆砌鲜艳色块打破体系一致的克制蓝调。"),
        b("brand-jira", "Jira", "Jira", "深蓝配状态色标签的严谨工程感", "#ffffff", "#172b4d", "#0052cc", "#00875a", "#f4f5f7", "#dfe1e6", "'Charlie Text',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "'Charlie Display','Charlie Text',sans-serif,'PingFang SC'", Small, Compact, "深蓝主色配状态标签色系，紧凑表格化排布服务于任务追踪效率。忌放大圆角与留白，削弱其严谨的工程管理气质。"),
        ],
    ));
    v.extend(cat(
        "设计 / 框架",
        vec![
        // ── 设计 / 创意平台 + UI 框架 ───────────────────────────
        b("brand-adobe", "Adobe", "Adobe Inc.", "红黑对比强烈的专业创意工具气质", "#ffffff", "#2c2c2c", "#fa0f00", "", "#f5f5f5", "#e0e0e0", "'Adobe Clean','Source Sans Pro',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Small, Normal, "红黑对比强烈奠定专业创作氛围，几何无衬线字体保持锐利现代。忌用糖果色或圆润卡通造型削弱专业质感。"),
        b("brand-behance", "Behance", "Behance (Adobe)", "深蓝主导作品网格化展示气质", "#ffffff", "#191919", "#0057ff", "", "#f5f5f5", "#e6e6e6", "-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Display, "深蓝为视觉锚点，大量留白与网格排版突出作品缩略图。忌高饱和多色堆砌喧宾夺主、抢过作品本身风头。"),
        b("brand-dribbble", "Dribbble", "Dribbble", "热粉高饱和的活泼社区创意氛围", "#ffffff", "#1a1a1a", "#ea4c89", "", "#fdf2f7", "#eaeaea", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "高饱和粉色点缀白底，圆润卡片与瀑布流营造活泼社区氛围。忌大面积低饱和灰调，显得沉闷缺乏设计感。"),
        b("brand-framer", "Framer", "Framer B.V.", "暗黑背景配电光蓝的极客未来感", "#0a0a0a", "#ffffff", "#0055ff", "", "#1a1a1a", "#2e2e2e", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Display, "深黑背景配电光蓝渐变强调色，纤细几何字体营造未来科技感。忌浅色背景与厚重衬线字体，削弱前沿气质。"),
        b("brand-webflow", "Webflow", "Webflow, Inc.", "克制蓝紫的专业建站工具质感", "#ffffff", "#0b0b0b", "#4353ff", "", "#f5f6ff", "#e2e4f5", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Display, "克制品牌蓝搭配白底与柔和阴影卡片，专业中带亲和力。忌堆叠多种高饱和渐变，破坏工具类产品的克制感。"),
        b("brand-canva", "Canva", "Canva Pty Ltd", "紫青渐变的友好易用创作氛围", "#ffffff", "#191919", "#8b3dff", "#00c4cc", "#f8f5ff", "#e6dcff", "'Canva Sans',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "紫色与青色渐变搭配圆润造型，营造友好易上手的创作氛围。忌使用生硬直角与冷淡纯灰，丢失亲和力。"),
        b("brand-sketch", "Sketch", "Sketch B.V.", "克制黑白配暖橙的工匠工具质感", "#ffffff", "#1c1c1e", "#fa7b17", "", "#f5f5f5", "#e5e5e5", "-apple-system,BlinkMacSystemFont,'SF Pro Text',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "黑白灰打底、暖橙点缀画布聚焦色，界面简洁强调专注创作。忌滥用高饱和多色分散注意力，偏离工匠工具定位。"),
        b("brand-pinterest", "Pinterest", "Pinterest, Inc.", "标志红配白的视觉发现导向", "#ffffff", "#211922", "#e60023", "", "#efefef", "#e9e9e9", "'Pinterest Sans',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Pill, Display, "醒目红为强调色，白底配圆润卡片瀑布流突出图片内容。忌大面积红色背景压过图片，喧宾夺主。"),
        b("brand-material", "Material Design", "Google Material Design", "紫色主导层次分明的谷歌质感", "#fffbfe", "#1c1b1f", "#6750a4", "#7d5260", "#e7e0ec", "#cac4d0", "'Roboto',-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif,'PingFang SC'", "'Google Sans','Roboto',-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif,'PingFang SC'", Medium, Normal, "紫色主色搭配分层表面色阶与柔和阴影，强调材质层级一致性。忌自定义色板打破色彩角色语义体系。"),
        b("brand-tailwind", "Tailwind CSS", "Tailwind Labs", "青蓝渐变极简实用的工程气质", "#ffffff", "#0f172a", "#06b6d4", "#3b82f6", "#f1f5f9", "#e2e8f0", "'Inter',ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "青蓝渐变点缀中性灰阶，排版克制注重可读性与工程感。忌堆砌多种高饱和色，破坏工具类产品的克制美学。"),
        b("brand-ant-design", "Ant Design", "Ant Group / Ant Design", "清爽蓝白的企业级中后台质感", "#ffffff", "#262626", "#1677ff", "#52c41a", "#fafafa", "#d9d9d9", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Compact, "拂晓蓝主色配浅灰中性面，界面紧凑严谨适合数据密集后台。忌使用花哨渐变色，削弱企业级产品的稳重感。"),
        b("brand-shadcn", "shadcn/ui", "shadcn/ui (open source)", "近黑白极简克制的现代工程感", "#ffffff", "#09090b", "#18181b", "", "#f4f4f5", "#e4e4e7", "'Geist','Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Small, Normal, "近乎无彩的黑白灰配色，靠间距与字重建立层次而非颜色。忌引入高饱和主题色，打破中性克制的工程美学。"),
        b("brand-bootstrap", "Bootstrap", "Bootstrap (open source)", "经典紫蓝稳健通用的组件质感", "#ffffff", "#212529", "#7952b3", "#0d6efd", "#f8f9fa", "#dee2e6", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "标志紫色配功能蓝，圆角适中风格稳健通用易识别。忌使用尖锐直角造型，削弱亲和的组件库气质。"),
        b("brand-chakra", "Chakra UI", "Chakra UI (open source)", "青紫搭配友好易上手组件气质", "#ffffff", "#1a202c", "#319795", "#805ad5", "#f7fafc", "#e2e8f0", "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "青绿配紫色渐变，圆润卡片与柔和阴影营造友好易用感。忌用生硬纯灰单色，丢失活泼的组件个性。"),
        b("brand-fluent", "Fluent Design", "Microsoft Fluent Design System", "微软蓝配浅灰的通透层次质感", "#ffffff", "#201f1e", "#0078d4", "", "#f3f2f1", "#edebe9", "'Segoe UI Variable','Segoe UI',-apple-system,BlinkMacSystemFont,Arial,sans-serif,'PingFang SC'", "'Segoe UI Variable Display','Segoe UI',-apple-system,BlinkMacSystemFont,Arial,sans-serif,'PingFang SC'", Small, Compact, "微软蓝主色配浅灰中性面，光效层次与细腻阴影营造通透感。忌使用厚重强阴影，破坏轻盈的层次质感。"),
        b("brand-ibm-carbon", "IBM Carbon", "IBM Carbon Design System", "克制蓝配纯直角的企业工程质感", "#ffffff", "#161616", "#0f62fe", "#8a3ffc", "#f4f4f4", "#e0e0e0", "'IBM Plex Sans',-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif,'PingFang SC'", "", Sharp, Compact, "IBM 蓝主色配零圆角直角造型，网格严谨排版紧凑克制。忌添加任何圆角柔化设计，破坏工业化企业气质。"),
        b("brand-mantine", "Mantine", "Mantine (open source)", "清爽蓝白的现代组件库质感", "#ffffff", "#1a1b1e", "#228be6", "", "#f1f3f5", "#dee2e6", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "克制的蓝色为主色，中性灰阶打底，圆角适中排版清晰现代。忌使用高饱和撞色组合，打破克制现代的组件质感。"),
        b("brand-radix", "Radix", "Radix UI / WorkOS", "极简黑白配紫的克制系统化质感", "#ffffff", "#211f26", "#6e56cf", "", "#f9f8f9", "#e4e2e4", "'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "黑白灰打底、紫罗兰作唯一强调色，克制且系统化。忌引入多个强调色并用，打破单一强调色的设计哲学。"),
        ],
    ));
    v.extend(cat(
        "社交 / 消费",
        vec![
        // ── 社交 / 消费级 ───────────────────────────────────────
        b("brand-instagram", "Instagram", "Instagram (Meta)", "渐变紫粉橙叠加简洁白底的生活感", "#ffffff", "#262626", "#c13584", "#f77737", "#fafafa", "#dbdbdb", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "以白底衬托渐变色标识与鲜艳图片内容，界面本身克制留白让内容唱主角。忌满屏铺渐变色块喧宾夺主，渐变只用于品牌标识与强调点。"),
        b("brand-x-twitter", "X", "X（原 Twitter）", "极简纯黑白高对比信息密度优先", "#000000", "#e7e9ea", "#ffffff", "", "#16181c", "#2f3336", "'Chirp',-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Pill, Compact, "黑白灰高对比塑造克制、直接、信息密度优先的气质，圆形头像与胶囊按钮是唯一的柔和元素。忌引入花哨强调色破坏纯粹的黑白基调。"),
        b("brand-tiktok", "TikTok", "TikTok（抖音海外版）", "黑底红青撞色故障感年轻躁动", "#000000", "#ffffff", "#fe2c55", "#25f4ee", "#121212", "#2f2f2f", "'TikTokFont',-apple-system,'PingFang SC','Helvetica Neue',Arial,sans-serif", "", Rounded, Display, "黑底上红青双色错位叠印制造故障感，全屏沉浸内容优先、UI 元素退居半透明浮层。忌把红青同时用于大面积色块，应保持细线或图标级点缀。"),
        b("brand-snapchat", "Snapchat", "Snapchat", "荧光黄配纯黑相机优先即时感", "#000000", "#ffffff", "#fffc00", "", "#14181a", "#2b2b2b", "'Graphik',-apple-system,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "荧光黄与纯黑的高饱和对比传递顽皮、即时、相机优先的气质，黄色只作点状高光而非背景。忌大面积铺黄底导致刺眼失衡。"),
        b("brand-facebook", "Facebook", "Facebook（Meta）", "蓝白经典组合的稳重社交基建感", "#ffffff", "#050505", "#1877f2", "", "#f0f2f5", "#ced0d4", "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "单一品牌蓝搭配浅灰卡片与白底，传递务实、稳重、老牌社交基建的既视感。忌为追求活力叠加高饱和多彩强调色，辨识度来自蓝白克制。"),
        b("brand-linkedin", "LinkedIn", "LinkedIn（Microsoft）", "深蓝配米白的职业化专业感", "#f4f2ee", "#191919", "#0a66c2", "", "#eef3f8", "#d9d9d9", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Small, Normal, "深蓝主色配米白背景与灰色卡片边框，营造职业社交场景的信任感与专业度。忌使用高饱和撞色或俏皮大圆角，削弱职场语境的严肃可信度。"),
        b("brand-reddit", "Reddit", "Reddit", "暗色论坛底配橙色符号的社区感", "#1a1a1b", "#d7dadc", "#ff4500", "#0079d3", "#272729", "#343536", "'IBM Plex Sans',-apple-system,Arial,sans-serif,'PingFang SC'", "", Pill, Compact, "深色论坛底色配标志性橙红与投票蓝双色系统，胶囊状投票标签强化社区讨论的密度感。忌把橙色用作大面积背景，应保留为图标与强调按钮点缀。"),
        b("brand-discord", "Discord", "Discord", "靛蓝紫配深灰底的游戏社群感", "#313338", "#dbdee1", "#5865f2", "", "#2b2d31", "#1e1f22", "'gg sans','Noto Sans',-apple-system,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Compact, "深灰多层级面板配标志性靛紫，服务器频道分栏结构传递游戏社群的活泼聚合感。忌用亮色背景替代深色主题，辨识度绑定深色与靛紫组合。"),
        b("brand-twitch", "Twitch", "Twitch（Amazon）", "紫黑配色的直播剧场沉浸感", "#0e0e10", "#efeff1", "#9146ff", "", "#18181c", "#26262c", "'Roobert','Helvetica Neue',Helvetica,Arial,sans-serif,'PingFang SC'", "", Small, Compact, "近黑背景配标志性紫色，营造直播剧场式的沉浸感，聊天区与视频区紧凑并置强调实时性。忌把紫色稀释为浅紫粉调，高饱和是关键辨识特征。"),
        b("brand-youtube", "YouTube", "YouTube（Google）", "黑红对比的影院感视频优先", "#0f0f0f", "#f1f1f1", "#ff0000", "", "#272727", "#3f3f3f", "'Roboto',Arial,sans-serif,'PingFang SC'", "", Pill, Normal, "深色影院式背景把红色压缩为品牌符号与关键按钮的点缀，最大化让视频画面成为视觉焦点。忌让红色蔓延成大面积背景色。"),
        b("brand-whatsapp", "WhatsApp", "WhatsApp（Meta）", "绿色系聊天气泡的温暖私密感", "#ece5dd", "#111b21", "#25d366", "#075e54", "#f0f0f0", "#d1d7db", "-apple-system,BlinkMacSystemFont,'Helvetica Neue',Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "米色聊天底纹配草绿与墨绿双层次品牌色，气泡化界面传递温暖朴素的私密对话感。忌用冷色调或纯白底替换米色底纹。"),
        b("brand-telegram", "Telegram", "Telegram", "天蓝纯净轻盈的速度技术感", "#ffffff", "#000000", "#229ed9", "#54c8ff", "#f4f4f5", "#dfe3e6", "-apple-system,BlinkMacSystemFont,Roboto,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "白底配纸飞机蓝的轻盈渐变，圆角气泡与简洁图标传递高速、纯净、极客友好的通讯气质。忌堆砌装饰性阴影或深色重底。"),
        b("brand-signal", "Signal", "Signal（Signal Foundation）", "冷静蓝白极简的隐私信任感", "#ffffff", "#000000", "#3a76f0", "", "#f5f5f5", "#e0e0e0", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "单一蓝色配大量留白与极简图标，刻意去装饰化传递隐私优先、值得信赖的克制气质。忌引入多彩强调色或营销感强的渐变。"),
        b("brand-threads", "Threads", "Threads（Meta）", "纯黑白极简文字流的内敛社交感", "#ffffff", "#000000", "#000000", "", "#f5f5f5", "#dbdbdb", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "黑白灰无彩色系统配纯文字信息流，界面近乎隐形以突出内容本身的克制气质。忌引入渐变色或高饱和强调色，差异化在于更极简。"),
        b("brand-bereal", "BeReal", "BeReal", "黑底双摄取景框的真实粗粝感", "#000000", "#ffffff", "#ffffff", "", "#1c1c1e", "#38383a", "-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "纯黑背景配白色文字与双摄取景框造型，传递反滤镜、反精修的真实粗粝气质。忌加入美颜级渐变或高光质感修饰。"),
        ],
    ));
    v.extend(cat(
        "媒体 / 电商",
        vec![
        // ── 媒体 / 流媒体 + 电商 / 出行 ─────────────────────────
        b("brand-spotify", "Spotify", "Spotify Technology S.A.", "黑底亮绿圆润活力的音乐感", "#121212", "#ffffff", "#1db954", "#1ed760", "#282828", "#3e3e3e", "'Circular Std','Helvetica Neue',Helvetica,Arial,'PingFang SC',sans-serif", "", Pill, Normal, "深黑背景搭配标志性亮绿与圆角胶囊按钮，营造沉浸随性的听歌氛围。忌把亮绿铺成大面积背景稀释其稀缺高亮属性。"),
        b("brand-netflix", "Netflix", "Netflix, Inc.", "纯黑底配戏剧红的高对比冲击", "#141414", "#ffffff", "#e50914", "#b1060f", "#221f1f", "#333333", "'Netflix Sans','Helvetica Neue',Helvetica,Arial,'PingFang SC',sans-serif", "'Bebas Neue','Helvetica Neue',Arial,'PingFang SC',sans-serif", Small, Display, "极简纯黑背景配单一戏剧红，字号大、留白克制，突出海报缩略图的视觉张力。忌把红色铺成大面积色块或降低黑白对比。"),
        b("brand-apple-music", "Apple Music", "Apple Inc.", "深色沉浸配粉红渐变的精致克制", "#0b0b0f", "#f5f5f7", "#fa233b", "#fb5c74", "#1c1c1e", "#2c2c2e", "-apple-system,BlinkMacSystemFont,'SF Pro Display','Helvetica Neue','PingFang SC',sans-serif", "", Medium, Normal, "深色磨砂背景叠加红粉渐变与专辑大图，字体走精致质感，讲究克制留白。忌堆砌装饰元素打破极简节奏。"),
        b("brand-disney-plus", "Disney+", "The Walt Disney Company", "深邃靛蓝底配亮蓝的奇幻仪式感", "#040714", "#ffffff", "#0063e5", "#01147c", "#101a33", "#232e47", "'Avenir Next','Helvetica Neue',Arial,'PingFang SC',sans-serif", "'Avenir Next Condensed','Avenir Next',Arial,'PingFang SC',sans-serif", Medium, Display, "深空靛蓝背景搭配星光蓝渐变与宏大海报网格，排版讲究仪式感。忌用冷灰中性色替代靛蓝稀释奇幻氛围。"),
        b("brand-hbo-max", "HBO Max", "Warner Bros. Discovery", "纯黑底配电光紫的精品剧集调性", "#000000", "#ffffff", "#7b2ff7", "#4d1a99", "#1a1a1a", "#2e2e2e", "-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Small, Display, "极简黑底衬托电光紫到品红的渐变光效，字体克制冷峻突显高端剧集调性。忌紫色饱和度不足显得廉价塑料感。"),
        b("brand-soundcloud", "SoundCloud", "SoundCloud Global Ltd.", "深色底配亮橙的声波街头感", "#121212", "#ffffff", "#ff5500", "#ff7700", "#232323", "#3a3a3a", "'Interstate','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Normal, "深灰黑背景配高饱和橙色波形与圆角标签，带社区涂鸦式活力。忌把橙色与红色混用造成信号混乱。"),
        b("brand-prime-video", "Prime Video", "Amazon.com, Inc.", "深蓝黑底配天蓝的影院沉浸感", "#0f171e", "#ffffff", "#00a8e1", "#1899d6", "#16232c", "#2c3e4c", "'Amazon Ember',Arial,'Helvetica Neue','PingFang SC',sans-serif", "", Small, Display, "深蓝黑背景搭配鲜明天蓝点缀，海报网格排布疏朗大气。忌用暖色调点缀破坏冷静的影院氛围。"),
        b("brand-shopify", "Shopify", "Shopify Inc.", "纯白底配草木绿的清爽商业信任感", "#ffffff", "#202223", "#96bf48", "#5e8e3e", "#f6f6f7", "#e1e3e5", "'Inter',-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Medium, Normal, "洁净留白背景配清新草木绿与圆润卡片，字体现代易读传递商家信赖感。忌绿色过饱和显得幼稚失去专业感。"),
        b("brand-amazon", "Amazon", "Amazon.com, Inc.", "白底配活力橙与深蓝的高效务实", "#ffffff", "#0f1111", "#ff9900", "#232f3e", "#f2f2f2", "#d5d9d9", "'Amazon Ember',Arial,'Helvetica Neue','PingFang SC',sans-serif", "", Small, Compact, "白底高密度信息排布配醒目橙色行动点与深蓝导航底色，讲求效率务实。忌橙色滥用到大面积背景失去导购焦点。"),
        b("brand-airbnb", "Airbnb", "Airbnb, Inc.", "白底配珊瑚红的圆润友好归属感", "#ffffff", "#222222", "#ff5a5f", "#00a699", "#f7f7f7", "#dddddd", "'Airbnb Cereal',-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Normal, "大量留白搭配温暖珊瑚红与青绿点缀、圆角卡片，传递友好归属的居家感。忌尖锐直角与冷色调破坏亲和力。"),
        b("brand-uber", "Uber", "Uber Technologies, Inc.", "黑白极简功能至上零装饰", "#ffffff", "#000000", "#000000", "", "#f6f6f6", "#e2e2e2", "'Uber Move','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Sharp, Compact, "纯黑白高对比、直角几何与紧凑网格，一切服务于路线与效率信息。忌引入彩色装饰或圆角柔化冲淡工具理性。"),
        b("brand-lyft", "Lyft", "Lyft, Inc.", "白底配亮粉的跳脱友善出行感", "#ffffff", "#05012c", "#ff00bf", "", "#f5f0f7", "#e6dbe9", "'Sofia Pro','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Pill, Normal, "明快白底配高饱和亮粉与胶囊按钮，传递轻松友善的城市出行感。忌粉色与其他鲜艳色并用造成视觉喧闹。"),
        b("brand-doordash", "DoorDash", "DoorDash, Inc.", "白底配警示红的紧凑高效外卖感", "#ffffff", "#191919", "#ff3008", "", "#f8f8f8", "#ebebeb", "-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Compact, "白底配高识别度警示红与紧凑圆角卡片，信息密度高强调即时性。忌红色大面积铺底引发视觉疲劳。"),
        b("brand-etsy", "Etsy", "Etsy, Inc.", "暖白底配手作橙的温暖匠人质感", "#ffffff", "#222222", "#f56400", "", "#fdf3ea", "#e4d9cd", "'Adjusted Etsy Circular','Circular Std','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Normal, "米白留白背景配温暖手作橙与圆润字体，传递独立匠人与手工温度感。忌冷灰科技感配色削弱这份温度。"),
        b("brand-ebay", "eBay", "eBay Inc.", "白底多彩字标的活泼市集感", "#ffffff", "#333333", "#e53238", "#0064d2", "#f5f5f5", "#e5e5e5", "'Market Sans','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Small, Compact, "白底配红蓝黄绿四色字标与紧凑列表，营造活泼多元的市集氛围。忌把四色平均滥用到界面各处失去焦点。"),
        b("brand-booking", "Booking.com", "Booking.com B.V.", "白底配深蓝与亮黄的高效决策感", "#ffffff", "#262626", "#003580", "#febb02", "#ebf3ff", "#d6e4f0", "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Arial,'PingFang SC',sans-serif", "", Small, Compact, "白底深蓝为主导航、亮黄作行动点缀，列表紧凑强调比价决策效率。忌深蓝大面积铺底显得沉闷压抑。"),
        b("brand-instacart", "Instacart", "Maplebear Inc. (Instacart)", "白底配新鲜绿与胡萝卜橙的鲜活感", "#ffffff", "#202124", "#43b02a", "#ff7009", "#f1faf0", "#dbeedd", "'Graphik','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Normal, "白底配清新绿与胡萝卜橙点缀、圆润卡片，传递生鲜到家的鲜活感。忌绿橙同时高饱和大面积并置显得杂乱。"),
        b("brand-deliveroo", "Deliveroo", "Deliveroo plc", "白底配薄荷青的清新快捷送餐感", "#ffffff", "#1b1a1a", "#00ccbc", "", "#e8f9f7", "#cdeeea", "'Deliveroo Sans','Helvetica Neue',Arial,'PingFang SC',sans-serif", "", Rounded, Compact, "白底配高识别度薄荷青与紧凑列表卡片，传递清新快捷的送餐效率。忌青色与其他冷色混用削弱品牌辨识度。"),
        ],
    ));
    v.extend(cat(
        "大厂 / 应用",
        vec![
        // ── 大厂 / 应用 ─────────────────────────────────────────
        b("brand-apple", "Apple", "Apple Inc.", "极简黑白灰的克制高级留白感", "#ffffff", "#1d1d1f", "#0071e3", "", "#f5f5f7", "#d2d2d7", "-apple-system,BlinkMacSystemFont,'SF Pro Text','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "-apple-system,BlinkMacSystemFont,'SF Pro Display','Helvetica Neue',Arial,sans-serif,'PingFang SC'", Rounded, Normal, "克制的黑白灰基调叠加单一强调蓝，依赖留白与精细字重传递高级感。忌堆砌多色装饰或粗重阴影破坏纯净感。"),
        b("brand-google", "Google", "Google LLC", "多彩活泼克制蓝色主导开放友好", "#ffffff", "#202124", "#4285f4", "#ea4335", "#f8f9fa", "#dadce0", "Roboto,'Google Sans Text',Arial,sans-serif,'PingFang SC'", "'Google Sans',Roboto,Arial,sans-serif,'PingFang SC'", Rounded, Normal, "四色标志活泼但界面本身克制，以蓝色为主导、大量白底与柔和阴影表达友好开放。忌把四色标志色平铺当作界面主色板滥用。"),
        b("brand-microsoft", "Microsoft", "Microsoft Corporation", "方格化蓝色系统的专业规整现代", "#ffffff", "#201f1e", "#0078d4", "#f25022", "#f3f2f1", "#edebe9", "'Segoe UI','Segoe UI Web',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "强调网格化排布与克制的蓝色系统色，圆角小而利落体现专业感。忌使用花哨渐变或过度圆润的糖果风格。"),
        b("brand-meta", "Meta", "Meta Platforms, Inc.", "深蓝圆润社交感的友好亲和", "#ffffff", "#050505", "#0064e0", "#0081fb", "#f0f2f5", "#dddfe2", "'Helvetica Neue',Helvetica,Arial,sans-serif,'PingFang SC'", "", Pill, Normal, "深蓝品牌色配大量白底与圆润卡片，字体友好圆融传递社交亲和感。忌用生硬直角或高饱和撞色破坏温和调性。"),
        b("brand-ibm", "IBM", "IBM Corporation", "方正蓝黑网格的工程严谨科技感", "#ffffff", "#161616", "#0f62fe", "#4589ff", "#f4f4f4", "#e0e0e0", "'IBM Plex Sans',Helvetica,Arial,sans-serif,'PingFang SC'", "", Sharp, Normal, "高对比蓝黑白网格排布，方正无圆角、单色块分区体现工程严谨。忌使用圆角气泡或柔和渐变冲淡科技感。"),
        b("brand-samsung", "Samsung", "Samsung Electronics", "深蓝金属质感的精密冷静工业风", "#ffffff", "#1c1c1c", "#1428a0", "", "#f5f5f5", "#e0e0e0", "'SamsungOne','Samsung Sharp Sans',Roboto,Arial,sans-serif,'PingFang SC'", "", Medium, Normal, "深蓝配大量黑白灰与金属质感留白，排版规整克制体现精密工业感。忌使用暖色系或手绘风格破坏冷静科技调性。"),
        b("brand-tesla", "Tesla", "Tesla, Inc.", "黑白极简配签名红的未来科技感", "#ffffff", "#171a20", "#e82127", "", "#f4f4f4", "#d9d9d9", "'Gotham SSm A','Gotham SSm B',Arial,sans-serif,'PingFang SC'", "", Sharp, Normal, "纯黑白极简底色上仅点缀签名红，依赖大留白与无衬线粗体传递未来感。忌多色堆砌或复杂装饰纹理。"),
        b("brand-nvidia", "Nvidia", "NVIDIA Corporation", "暗黑底配荧光绿的硬核算力感", "#000000", "#ffffff", "#76b900", "", "#1a1a1a", "#333333", "'NVIDIA Sans',Arial,sans-serif,'PingFang SC'", "", Sharp, Display, "黑底配荧光绿的高对比暗色主题，字体粗壮几何强调算力与未来感。忌在浅色亮白背景上使用，丢失品牌识别度。"),
        b("brand-intel", "Intel", "Intel Corporation", "蓝色几何工业感的紧凑精密", "#ffffff", "#000000", "#0068b5", "#00c7fd", "#f5f5f5", "#e2e2e2", "'IntelOne Display','IntelOne Text',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "蓝色系配浅灰底与几何图形，排版紧凑体现半导体精密感。忌使用暖色或有机曲线打破工业几何调性。"),
        b("brand-salesforce", "Salesforce", "Salesforce, Inc.", "云朵蓝圆润友好的企业云服务感", "#ffffff", "#032e61", "#00a1e0", "#032d60", "#f3f6f9", "#dddbda", "'Salesforce Sans','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "云朵蓝配柔和圆角卡片与浅灰底，字体圆润友好体现企业云服务的亲和力。忌用尖锐直角或冷峻纯黑白破坏云端轻盈感。"),
        b("brand-oracle", "Oracle", "Oracle Corporation", "砖红沉稳保守的企业级可靠感", "#ffffff", "#312d2a", "#c74634", "", "#f5f5f5", "#e1e1e1", "'Oracle Sans','Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "砖红色配沉稳深灰文字与浅灰底，排版规整偏保守体现企业级可靠感。忌用鲜亮糖果色或圆润卡通元素。"),
        b("brand-sap", "SAP", "SAP SE", "亮蓝深蓝网格的理性高效企业感", "#ffffff", "#32363a", "#0070f2", "#003366", "#f5f6f7", "#d9d9d9", "'72','72full',Arial,sans-serif,'PingFang SC'", "", Small, Normal, "亮蓝配深蓝辅助色与浅灰网格，排版紧凑体现企业软件的专业与效率。忌使用手绘风格或高饱和暖色破坏理性调性。"),
        b("brand-atlassian", "Atlassian", "Atlassian Corporation", "蓝紫圆角友好的协作现代感", "#ffffff", "#172b4d", "#0052cc", "#2684ff", "#f4f5f7", "#dfe1e6", "'Charlie Text','Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "'Charlie Display','Charlie Text',Arial,sans-serif,'PingFang SC'", Rounded, Normal, "蓝紫色系配浅灰底与圆角卡片，字体现代圆润体现协作友好。忌使用尖锐直角或工业冷灰破坏协作亲和感。"),
        b("brand-duolingo", "Duolingo", "Duolingo, Inc.", "荧光绿胶囊圆角的游戏化欢乐感", "#ffffff", "#4b4b4b", "#58cc02", "#1cb0f6", "#f7f7f7", "#e5e5e5", "'Feather Bold','DIN Next Rounded',Arial,sans-serif,'PingFang SC'", "", Pill, Display, "荧光绿配天蓝辅助色与大量圆角胶囊元素，字体粗圆活泼体现游戏化学习。忌使用严肃衬线字体或低饱和灰调破坏欢乐感。"),
        b("brand-grammarly", "Grammarly", "Grammarly, Inc.", "薄荷绿清爽现代的智能写作感", "#ffffff", "#15232d", "#15c39a", "#114f3e", "#f4faf8", "#e0e6e3", "Graphik,'Helvetica Neue',Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "薄荷绿配深墨绿文字与柔和留白，排版清爽现代体现智能写作助手调性。忌使用高饱和暖色或生硬直角破坏温和专业感。"),
        b("brand-1password", "1Password", "1Password (AgileBits Inc.)", "深蓝简洁圆角的安全可信克制感", "#ffffff", "#1a1a1a", "#0364d3", "#0090ff", "#f5f7fa", "#e1e5ea", "Inter,-apple-system,'Segoe UI',Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "深邃蓝配简洁圆角卡片与浅灰底，排版克制体现安全可信。忌使用鲜亮警示色或复杂装饰削弱信任感。"),
        b("brand-arc-browser", "Arc Browser", "The Browser Company", "多彩渐变大圆角的创意活力个性", "#ffffff", "#1b1b1b", "#7f5af0", "#ff5c74", "#f6f3ff", "#e6e0fa", "Inter,-apple-system,'Segoe UI',Arial,sans-serif,'PingFang SC'", "", Rounded, Display, "多彩渐变配柔和大圆角与充足留白，字体现代圆润体现创意个性化。忌使用单调纯黑白或尖锐直角破坏活力调性。"),
        b("brand-brave", "Brave", "Brave Software, Inc.", "橙红狮子标志的隐私速度简洁感", "#ffffff", "#1e1e1e", "#fb542b", "", "#f7f7f7", "#e2e2e2", "Poppins,Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "橙红狮子标志配克制留白，字体现代体现隐私与速度感。忌使用花哨渐变或过多装饰破坏简洁高效印象。"),
        b("brand-firefox", "Firefox", "Mozilla Firefox", "火焰橙渐变紫色的开放温暖感", "#ffffff", "#0c0c0d", "#ff7139", "#9059ff", "#f9f9fb", "#e0e0e6", "-apple-system,'Segoe UI',Helvetica,Arial,sans-serif,'PingFang SC'", "'Zilla Slab',Georgia,serif,'PingFang SC'", Rounded, Normal, "火焰橙渐变配紫色辅助色，标题偏衬线粗体体现开放与活力。忌整体转为纯企业蓝灰，丢失温暖开源气质。"),
        b("brand-opera", "Opera", "Opera Software", "鲜红简洁轻快的现代浏览器感", "#ffffff", "#1b1b1b", "#ff1b2d", "", "#fff2f2", "#ffd6d6", "'Motiva Sans',Arial,sans-serif,'PingFang SC'", "", Rounded, Normal, "鲜红色配简洁白底与浅灰卡片，圆角适中体现浏览器的轻快现代。忌使用暗色沉重底色或复杂纹理破坏轻盈感。"),
        ],
    ));
    v
}

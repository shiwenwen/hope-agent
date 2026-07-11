//! 导出辅助（后端）：把前端栅格化出的整页 PNG 组装成 **PPTX**（确定性 OOXML zip，
//! 复用 ha-core 既有 `zip` 依赖，零新增重依赖）。
//!
//! PNG/PDF 走前端客户端栅格化（html2canvas + jspdf，非打断、两模式通用）；PPTX 需
//! zip 打包，故前端把每页 PNG（base64）传后端由此构建。每页 = 一张铺满幻灯片的图片。

use anyhow::{Context, Result};
use std::io::Write;
use zip::write::SimpleFileOptions;

/// 单页图片（前端 html2canvas 产出）。
pub struct SlideImage {
    pub png: Vec<u8>,
}

/// 幻灯片 EMU 尺寸（16:9，13.333in × 7.5in）。
const SLIDE_W_EMU: i64 = 12_192_000;
const SLIDE_H_EMU: i64 = 6_858_000;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 构建一个每页铺满整图的 PPTX。返回 pptx 字节。
pub fn build_pptx(slides: &[SlideImage], title: &str) -> Result<Vec<u8>> {
    if slides.is_empty() {
        anyhow::bail!("no slides to export");
    }
    let n = slides.len();
    let buf = Vec::new();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(buf));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let w = |name: &str,
             data: &[u8],
             zip: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>|
     -> Result<()> {
        zip.start_file(name, opts)?;
        zip.write_all(data)?;
        Ok(())
    };

    // [Content_Types].xml
    let mut ct = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>",
    );
    for i in 1..=n {
        ct.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    ct.push_str("</Types>");
    w("[Content_Types].xml", ct.as_bytes(), &mut zip)?;

    // _rels/.rels
    w(
        "_rels/.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
</Relationships>",
        &mut zip,
    )?;

    // ppt/presentation.xml
    let mut pres = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst>\
<p:sldIdLst>",
    );
    for i in 1..=n {
        // slide relationship ids start at rId2 (rId1 = master)
        pres.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            255 + i,
            i + 1
        ));
    }
    pres.push_str(&format!(
        "</p:sldIdLst><p:sldSz cx=\"{SLIDE_W_EMU}\" cy=\"{SLIDE_H_EMU}\"/>\
<p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    ));
    w("ppt/presentation.xml", pres.as_bytes(), &mut zip)?;

    // ppt/_rels/presentation.xml.rels
    let mut prels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>\
<Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>",
    );
    for i in 1..=n {
        prels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{i}.xml\"/>",
            i + 1
        ));
    }
    prels.push_str("</Relationships>");
    w(
        "ppt/_rels/presentation.xml.rels",
        prels.as_bytes(),
        &mut zip,
    )?;

    // Theme (minimal)
    w("ppt/theme/theme1.xml", THEME_XML.as_bytes(), &mut zip)?;

    // Slide master + layout (minimal, references theme)
    w(
        "ppt/slideMasters/slideMaster1.xml",
        slide_master_xml().as_bytes(),
        &mut zip,
    )?;
    w(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/>\
</Relationships>",
        &mut zip,
    )?;
    w(
        "ppt/slideLayouts/slideLayout1.xml",
        slide_layout_xml().as_bytes(),
        &mut zip,
    )?;
    w(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>\
</Relationships>",
        &mut zip,
    )?;

    // Slides + media
    for (idx, slide) in slides.iter().enumerate() {
        let i = idx + 1;
        w(&format!("ppt/media/image{i}.png"), &slide.png, &mut zip)?;
        w(
            &format!("ppt/slides/slide{i}.xml"),
            slide_xml(i, title).as_bytes(),
            &mut zip,
        )?;
        w(
            &format!("ppt/slides/_rels/slide{i}.xml.rels"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rIdImg\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{i}.png\"/>\
</Relationships>"
            )
            .as_bytes(),
            &mut zip,
        )?;
    }

    let cursor = zip.finish().context("finalize pptx zip")?;
    Ok(cursor.into_inner())
}

/// 结构化 PPTX 的一页大纲（标题 + 要点，可编辑文本）。
pub struct SlideOutline {
    pub title: String,
    pub bullets: Vec<String>,
}

/// 构建**可编辑文本** PPTX（每页 标题框 + 要点框，从 deck 大纲派生）。与 `build_pptx`（整图）
/// 双模式：这里产原生文本 shape（PowerPoint 里可直接改字），无位图、无 media 部件。
pub fn build_pptx_outline(slides: &[SlideOutline], deck_title: &str) -> Result<Vec<u8>> {
    if slides.is_empty() {
        anyhow::bail!("no slides to export");
    }
    let n = slides.len();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let w = |name: &str,
             data: &[u8],
             zip: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>|
     -> Result<()> {
        zip.start_file(name, opts)?;
        zip.write_all(data)?;
        Ok(())
    };

    let mut ct = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>",
    );
    for i in 1..=n {
        ct.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    ct.push_str("</Types>");
    w("[Content_Types].xml", ct.as_bytes(), &mut zip)?;

    w(
        "_rels/.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
</Relationships>",
        &mut zip,
    )?;

    let mut pres = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst>\
<p:sldIdLst>",
    );
    for i in 1..=n {
        pres.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1));
    }
    pres.push_str(&format!(
        "</p:sldIdLst><p:sldSz cx=\"{SLIDE_W_EMU}\" cy=\"{SLIDE_H_EMU}\"/>\
<p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    ));
    w("ppt/presentation.xml", pres.as_bytes(), &mut zip)?;

    let mut prels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>\
<Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>",
    );
    for i in 1..=n {
        prels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{i}.xml\"/>",
            i + 1
        ));
    }
    prels.push_str("</Relationships>");
    w("ppt/_rels/presentation.xml.rels", prels.as_bytes(), &mut zip)?;

    w("ppt/theme/theme1.xml", THEME_XML.as_bytes(), &mut zip)?;
    w(
        "ppt/slideMasters/slideMaster1.xml",
        slide_master_xml().as_bytes(),
        &mut zip,
    )?;
    w(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/>\
</Relationships>",
        &mut zip,
    )?;
    w(
        "ppt/slideLayouts/slideLayout1.xml",
        slide_layout_xml().as_bytes(),
        &mut zip,
    )?;
    w(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>\
</Relationships>",
        &mut zip,
    )?;

    let _ = deck_title;
    for (idx, slide) in slides.iter().enumerate() {
        let i = idx + 1;
        w(
            &format!("ppt/slides/slide{i}.xml"),
            outline_slide_xml(slide).as_bytes(),
            &mut zip,
        )?;
        w(
            &format!("ppt/slides/_rels/slide{i}.xml.rels"),
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
</Relationships>",
            &mut zip,
        )?;
    }

    let cursor = zip.finish().context("finalize pptx zip")?;
    Ok(cursor.into_inner())
}

/// 文本 slide XML：标题框（上）+ 要点框（下），显式 xfrm 定位（不依赖占位符）。
fn outline_slide_xml(slide: &SlideOutline) -> String {
    let title_runs = if slide.title.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<a:p><a:r><a:rPr lang=\"en-US\" sz=\"3600\" b=\"1\"/><a:t>{}</a:t></a:r></a:p>",
            esc(&slide.title)
        )
    };
    let body_runs: String = if slide.bullets.is_empty() {
        "<a:p><a:endParaRPr/></a:p>".to_string()
    } else {
        slide
            .bullets
            .iter()
            .map(|b| {
                format!(
                    "<a:p><a:pPr><a:buChar char=\"•\"/></a:pPr>\
<a:r><a:rPr lang=\"en-US\" sz=\"2000\"/><a:t>{}</a:t></a:r></a:p>",
                    esc(b)
                )
            })
            .collect()
    };
    // 标题框：0.75in 边距，顶部；内容框：其下铺满。
    let margin: i64 = 685_800;
    let box_w = SLIDE_W_EMU - margin * 2;
    let title_y: i64 = 400_000;
    let title_h: i64 = 1_000_000;
    let body_y = title_y + title_h + 200_000;
    let body_h = SLIDE_H_EMU - body_y - margin;
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>\
<p:nvPr/></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"{margin}\" y=\"{title_y}\"/><a:ext cx=\"{box_w}\" cy=\"{title_h}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
<p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>{title_runs}</p:txBody></p:sp>\
<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Content\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>\
<p:nvPr/></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"{margin}\" y=\"{body_y}\"/><a:ext cx=\"{box_w}\" cy=\"{body_h}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
<p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>{body_runs}</p:txBody></p:sp>\
</p:spTree></p:cSld><p:clrMapOvr><a:overrideClrMapping bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" \
accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" \
accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/></p:clrMapOvr></p:sld>"
    )
}

/// 单页 slide XML：一张铺满整片的图片。
fn slide_xml(i: usize, title: &str) -> String {
    let alt = esc(&format!("{title} — {i}"));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"{alt}\" descr=\"{alt}\"/>\
<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rIdImg\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{SLIDE_W_EMU}\" cy=\"{SLIDE_H_EMU}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:overrideClrMapping bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" \
accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" \
accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/></p:clrMapOvr></p:sld>"
    )
}

fn slide_master_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" \
accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    )
}

fn slide_layout_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\">\
<p:cSld name=\"Blank\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>\
<p:clrMapOvr><a:overrideClrMapping bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" \
accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" \
hlink=\"hlink\" folHlink=\"folHlink\"/></p:clrMapOvr></p:sldLayout>"
        .to_string()
}

const THEME_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office\">\
<a:themeElements><a:clrScheme name=\"Office\">\
<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"44546A\"/></a:dk2><a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1><a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2>\
<a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3><a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4>\
<a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5><a:accent6><a:srgbClr val=\"70AD47\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Office\"><a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Office\"><a:fillStyleLst>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle>\
<a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>";

// ── ZIP export ──────────────────────────────────────────────────────

/// One artifact's files for a ZIP bundle.
pub struct ZipArtifact {
    /// Folder inside the archive (safe slug). Empty = archive root (single export).
    pub folder: String,
    /// Clean self-contained `index.html`.
    pub html: String,
    /// `(body, css, js)` source; `Some` bundles a `source/` dir (single export).
    pub source: Option<(String, String, String)>,
    pub title: String,
    pub kind: String,
}

fn zip_write(
    zip: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
    opts: SimpleFileOptions,
    name: &str,
    data: &[u8],
) -> Result<()> {
    zip.start_file(name, opts)?;
    zip.write_all(data)?;
    Ok(())
}

/// Build a ZIP from artifacts. `index_html` (Some) is written at the archive root as
/// a gallery/manifest (project export); each artifact lands under its `folder`.
/// Reuses the existing `zip` dependency — no new deps.
pub fn build_zip(artifacts: &[ZipArtifact], index_html: Option<&str>) -> Result<Vec<u8>> {
    if artifacts.is_empty() {
        anyhow::bail!("nothing to export");
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(idx) = index_html {
        zip_write(&mut zip, opts, "index.html", idx.as_bytes())?;
    }
    for a in artifacts {
        let base = if a.folder.is_empty() {
            String::new()
        } else {
            format!("{}/", a.folder)
        };
        zip_write(
            &mut zip,
            opts,
            &format!("{base}index.html"),
            a.html.as_bytes(),
        )?;
        if let Some((body, css, js)) = &a.source {
            zip_write(
                &mut zip,
                opts,
                &format!("{base}source/body.html"),
                body.as_bytes(),
            )?;
            zip_write(
                &mut zip,
                opts,
                &format!("{base}source/style.css"),
                css.as_bytes(),
            )?;
            zip_write(
                &mut zip,
                opts,
                &format!("{base}source/script.js"),
                js.as_bytes(),
            )?;
        }
        let readme = format!(
            "# {title}\n\n- kind: {kind}\n\n自包含产物：在浏览器直接打开 `index.html` 即可（零外部依赖）。\
源码见 `source/`。\n",
            title = a.title,
            kind = a.kind,
        );
        zip_write(
            &mut zip,
            opts,
            &format!("{base}README.md"),
            readme.as_bytes(),
        )?;
    }
    Ok(zip.finish()?.into_inner())
}

/// 从任意「归档内路径 → 字节」列表构建 ZIP（通用，供代码交付包等自定义打包用）。
pub fn build_files_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    if files.is_empty() {
        anyhow::bail!("nothing to zip");
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        zip_write(&mut zip, opts, name, data)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_files_zip_roundtrip() {
        let files = vec![
            ("index.html".to_string(), b"<h1>x</h1>".to_vec()),
            ("tokens/tokens.css".to_string(), b":root{}".to_vec()),
        ];
        let bytes = build_files_zip(&files).unwrap();
        let mut r = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.by_name("tokens/tokens.css").is_ok());
        assert!(build_files_zip(&[]).is_err());
    }

    #[test]
    fn build_zip_single_and_project() {
        let a = ZipArtifact {
            folder: String::new(),
            html: "<!doctype html><h1>A</h1>".into(),
            source: Some(("<h1>A</h1>".into(), ".x{}".into(), "".into())),
            title: "Deck A".into(),
            kind: "deck".into(),
        };
        let single = build_zip(std::slice::from_ref(&a), None).unwrap();
        assert_eq!(&single[0..2], b"PK");
        let r = zip::ZipArchive::new(std::io::Cursor::new(single)).unwrap();
        let names: Vec<String> = r.file_names().map(str::to_string).collect();
        assert!(names.iter().any(|n| n == "index.html"));
        assert!(names.iter().any(|n| n == "source/body.html"));
        assert!(names.iter().any(|n| n == "README.md"));

        let proj = build_zip(
            &[ZipArtifact {
                folder: "deck-a-1234abcd".into(),
                html: "<!doctype html><h1>A</h1>".into(),
                source: None,
                title: "Deck A".into(),
                kind: "deck".into(),
            }],
            Some("<!doctype html><h1>Gallery</h1>"),
        )
        .unwrap();
        let r2 = zip::ZipArchive::new(std::io::Cursor::new(proj)).unwrap();
        let names2: Vec<String> = r2.file_names().map(str::to_string).collect();
        assert!(names2.iter().any(|n| n == "index.html"));
        assert!(names2.iter().any(|n| n == "deck-a-1234abcd/index.html"));
    }

    #[test]
    fn build_zip_empty_errors() {
        assert!(build_zip(&[], None).is_err());
    }

    #[test]
    fn build_pptx_produces_valid_zip() {
        // 1×1 PNG。
        let png = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let slides = vec![SlideImage { png: png.clone() }, SlideImage { png }];
        let bytes = build_pptx(&slides, "Test Deck").unwrap();
        // ZIP 魔数 PK\x03\x04。
        assert_eq!(&bytes[0..2], b"PK");
        // 可被 zip reader 打开 + 含关键部件。
        let reader = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<String> = reader.file_names().map(str::to_string).collect();
        assert!(names.iter().any(|n| n == "[Content_Types].xml"));
        assert!(names.iter().any(|n| n == "ppt/presentation.xml"));
        assert!(names.iter().any(|n| n == "ppt/slides/slide1.xml"));
        assert!(names.iter().any(|n| n == "ppt/slides/slide2.xml"));
        assert!(names.iter().any(|n| n == "ppt/media/image1.png"));
    }

    #[test]
    fn empty_slides_errors() {
        assert!(build_pptx(&[], "x").is_err());
    }

    #[test]
    fn build_pptx_outline_produces_editable_text_zip() {
        let slides = vec![
            SlideOutline {
                title: "封面 & <预告>".into(),
                bullets: vec!["要点一".into(), "要点二".into()],
            },
            SlideOutline {
                title: "第二页".into(),
                bullets: vec![],
            },
        ];
        let bytes = build_pptx_outline(&slides, "Deck").unwrap();
        assert_eq!(&bytes[0..2], b"PK");
        let mut reader = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<String> = reader.file_names().map(str::to_string).collect();
        assert!(names.iter().any(|n| n == "ppt/slides/slide1.xml"));
        assert!(names.iter().any(|n| n == "ppt/slides/slide2.xml"));
        // 无 media 部件（纯文本）。
        assert!(!names.iter().any(|n| n.starts_with("ppt/media/")));
        // slide1 含转义后的标题文本 + 要点。
        let mut s1 = String::new();
        use std::io::Read;
        reader
            .by_name("ppt/slides/slide1.xml")
            .unwrap()
            .read_to_string(&mut s1)
            .unwrap();
        assert!(s1.contains("封面 &amp; &lt;预告&gt;"));
        assert!(s1.contains("要点一"));
        assert!(s1.contains("<a:buChar"));
    }

    #[test]
    fn build_pptx_outline_empty_errors() {
        assert!(build_pptx_outline(&[], "x").is_err());
    }
}

#!/usr/bin/env python3
"""Generate a synthetic EPUB with one very large chapter (TODO #55 sample).

Usage: python3 scripts/make-big-chapter-epub.py [paragraphs] [output.epub]
Default: 4000 paragraphs -> ~465KB single-chapter HTML.
"""
import os
import sys
import zipfile

paras = int(sys.argv[1]) if len(sys.argv) > 1 else 4000
out = sys.argv[2] if len(sys.argv) > 2 else "/tmp/big-chapter.epub"

para = "<p>大章节压力测试段落。The quick brown fox jumps over the lazy dog. 静电鱼说：知识就是力量。</p>\n"
xhtml = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Big Chapter</title></head>\n'
    f"<body>{para * paras}</body></html>"
)

container = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""

opf = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Big Chapter Test</dc:title><dc:identifier id="id">big-1</dc:identifier>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>
"""

with zipfile.ZipFile(out, "w") as z:
    z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
    z.writestr("META-INF/container.xml", container)
    z.writestr("OEBPS/content.opf", opf)
    z.writestr("OEBPS/ch1.xhtml", xhtml)

print(f"{out}: {os.path.getsize(out)} bytes, {paras} paragraphs")

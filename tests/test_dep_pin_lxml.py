#!/usr/bin/env python
"""lxml dependency pinning.

backend 사용처:
- backend/book_manager.py:126,270,303,576  EPUB OPF/NCX XML 파싱·조작
    from lxml import etree
    opf = etree.fromstring(data, etree.XMLParser(recover=True))
    spine = opf.find(f".//{{{ns}}}spine")
    items = el.findall(f"{{{ns}}}itemref")  / it.get("idref")
    new = etree.SubElement(parent, tag); new.set(k, v)
    etree.tostring(opf, encoding="unicode")
    parent.remove(child) / child.getparent() / opf.iter(tag)

박제 API:
- etree.fromstring(bytes, etree.XMLParser(recover=True))
- element.find / findall (namespace 포함) / get / set / remove / getparent / iter
- etree.SubElement(parent, tag)
- etree.tostring(el, encoding="unicode") -> str
"""

import unittest


class TestLxmlDependencyPinning(unittest.TestCase):
    NS = "http://www.idpf.org/2007/opf"

    OPF_XML = b"""<?xml version="1.0" encoding="UTF-8"?>
    <package xmlns="http://www.idpf.org/2007/opf" version="3.0">
        <manifest>
            <item id="chap01" href="chap01.xhtml"/>
            <item id="chap02" href="chap02.xhtml"/>
        </manifest>
        <spine toc="ncx">
            <itemref idref="chap01"/>
            <itemref idref="chap02"/>
        </spine>
    </package>
    """

    def _parse(self):
        from lxml import etree

        return etree.fromstring(self.OPF_XML, etree.XMLParser(recover=True))

    def test_etree_importable(self):
        """from lxml import etree (book_manager.py가 의존하는 진입점)"""
        from lxml import etree

        self.assertTrue(hasattr(etree, "fromstring"))
        self.assertTrue(hasattr(etree, "SubElement"))
        self.assertTrue(hasattr(etree, "tostring"))
        self.assertTrue(hasattr(etree, "XMLParser"))

    def test_fromstring_with_recover_parser(self):
        """etree.fromstring(data, XMLParser(recover=True))"""
        opf = self._parse()
        self.assertIsNotNone(opf)

    def test_find_findall_get_with_namespace(self):
        """namespace 포함 find/findall + get() 속성"""
        opf = self._parse()
        spine = opf.find(f"{{{self.NS}}}spine")
        self.assertIsNotNone(spine)
        self.assertEqual(spine.get("toc"), "ncx")
        refs = spine.findall(f"{{{self.NS}}}itemref")
        self.assertEqual(len(refs), 2)
        self.assertEqual(refs[0].get("idref"), "chap01")

    def test_subelement_set_and_tostring(self):
        """etree.SubElement + set() 후 tostring(encoding='unicode')"""
        from lxml import etree

        opf = self._parse()
        spine = opf.find(f"{{{self.NS}}}spine")
        new_ref = etree.SubElement(spine, f"{{{self.NS}}}itemref")
        new_ref.set("idref", "chap_new")
        result = etree.tostring(opf, encoding="unicode")
        self.assertIsInstance(result, str)
        self.assertIn("chap_new", result)

    def test_remove_getparent_iter(self):
        """remove / getparent / iter (spine·NCX 조작 패턴)"""
        opf = self._parse()
        spine = opf.find(f"{{{self.NS}}}spine")
        refs = spine.findall(f"{{{self.NS}}}itemref")
        ref = refs[0]
        self.assertIs(ref.getparent(), spine)
        spine.remove(ref)
        self.assertEqual(len(spine.findall(f"{{{self.NS}}}itemref")), 1)
        items = list(opf.iter(f"{{{self.NS}}}item"))
        self.assertEqual(len(items), 2)

    def test_delete_attribute(self):
        """del el.attrib[key] (spine toc 속성 제거 패턴)"""
        opf = self._parse()
        spine = opf.find(f"{{{self.NS}}}spine")
        del spine.attrib["toc"]
        self.assertIsNone(spine.get("toc"))


if __name__ == "__main__":
    unittest.main()

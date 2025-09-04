#!/usr/bin/env python3
"""
Bookstore 클래스 - 온라인 서점에서 도서 정보를 검색하는 기능을 제공합니다.
"""

import re
import requests
from urllib.parse import quote, urljoin
from typing import Tuple, Optional, List, Dict
from bs4 import BeautifulSoup
import time
import sys
import os
import logging
import json
import http.client
http.client._MAXHEADERS = 1000  # allow more response headers
from abc import ABC, abstractmethod
import uuid

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# 추상 베이스 클래스
class AbstractBookstore(ABC):
    """서점 검색을 위한 베이스 인터페이스"""
    BASE_URL: str
    MAX_RESULTS: int = 2

    def __init__(self, base_dir: str = '.', verbose: bool = True):
        self.base_dir = base_dir
        self.verbose = verbose
        # 세션/헤더 초기화
        self.session = requests.Session()
        self.session.headers.update({
            'User-Agent': 'curl/7.79.1',
            'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
            'Accept-Language': 'ko-KR,ko;q=0.9,en-US;q=0.8,en;q=0.7',
            'Accept-Encoding': 'gzip, deflate',
            'Referer': self.BASE_URL
        })

    @abstractmethod
    def build_search_url(self, keyword: str) -> str:
        """검색 URL 구성"""
        pass

    @abstractmethod
    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        """검색 결과 링크 추출"""
        pass

    @abstractmethod
    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        """상세 페이지에서 책 정보 추출"""
        pass

    def search_by_keyword(self, keyword: str) -> List[Tuple[str, str, str, str, str]]:
        url = self.build_search_url(keyword)
        # 검색 페이지 요청 예외 처리
        try:
            resp = self.session.get(url, timeout=10, verify=False)
        except Exception as e:
            logger.error(f"검색 페이지 요청 실패: {e}")
            return []
        resp.encoding = 'utf-8'
        soup = BeautifulSoup(resp.text, 'html.parser')
        links = self.extract_search_links(soup)
        results: List[Tuple[str, str, str, str, str]] = []
        for detail_url in links[:self.MAX_RESULTS]:
            # 캐시된 HTML 로드 시도
            html = self._load_html_from_tmp(detail_url)
            if html is None:
                # 캐시가 없으면 HTTP 요청 후 저장
                try:
                    resp2 = self.session.get(detail_url, timeout=10, verify=False)
                except Exception as e:
                    logger.error(f"상세 페이지 요청 실패: {detail_url} - {e}")
                    continue
                resp2.encoding = 'utf-8'
                html = resp2.text
                self._save_html_to_tmp(html, detail_url)
            if not html or not html.strip():
                if self.verbose:
                    logger.warning(f"상세 페이지가 비어있습니다: {detail_url}")
                continue
            detail_soup = BeautifulSoup(html, 'html.parser')
            info = self.extract_book_info(detail_soup)
            title = info.get('title', '')
            author = info.get('author', '')
            category = info.get('category', '')
            # 카테고리 기본 처리 (비디오/판타지)
            if category:
                parts = [p.strip() for p in category.split('>')]
                category = ' > '.join(parts[:3])
            results.append((title, author, category, detail_url, url))
        return results

    def _save_html_to_tmp(self, html: str, url: str):
        try:
            # URL 기반 deterministic UUID 생성
            filename = f"{uuid.uuid5(uuid.NAMESPACE_URL, url)}.html"
            path = os.path.join('/tmp', filename)
            with open(path, 'w', encoding='utf-8') as f:
                f.write(html)
            if self.verbose:
                logger.info(f"Saved HTML to {path}")
        except Exception as e:
            logger.error(f"Failed to save HTML to tmp: {e}")

    def _load_html_from_tmp(self, url: str) -> Optional[str]:
        """
        주어진 URL에 해당하는 임시 저장된 HTML을 로드합니다. 존재하지 않으면 None을 반환합니다.
        """
        try:
            filename = f"{uuid.uuid5(uuid.NAMESPACE_URL, url)}.html"
            path = os.path.join('/tmp', filename)
            if os.path.exists(path):
                if self.verbose:
                    logger.info(f"Loading cached HTML from {path}")
                with open(path, 'r', encoding='utf-8') as f:
                    return f.read()
        except Exception as e:
            logger.error(f"Failed to load HTML from tmp: {e}")
        return None

# Yes24 구현
class Bookstore(AbstractBookstore):
    BASE_URL = 'https://www.yes24.com'

    def build_search_url(self, keyword: str) -> str:
        encoded = quote(keyword)
        return f"{self.BASE_URL}/Product/Search?domain=ALL&query={encoded}"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        """yes24 검색 결과 페이지에서 상세 페이지 링크를 CSS selector로 추출합니다."""
        # 1) 계층적 선택자 우선 사용
        selector = (
            'ul#yesSchList > li > div.itemUnit > div.item_info > '
            'div.info_row.info_name a.gd_name[href^="/product/goods/"]'
        )
        links: List[str] = []
        seen = set()
        for a_tag in soup.select(selector):
            href = a_tag['href']
            full = urljoin(self.BASE_URL, href)
            if full not in seen:
                seen.add(full)
                links.append(full)
        # 2) 계층적 방식이 실패할 경우 fallback
        if not links:
            if self.verbose:
                logger.info("계층적 방식으로 링크 추출 실패, fallback 로직 실행")
            for a_tag in soup.find_all('a', class_='gd_name', href=True):
                href = a_tag['href']
                if href.startswith('/product/goods/'):
                    full = urljoin(self.BASE_URL, href)
                    if full not in seen:
                        seen.add(full)
                        links.append(full)
        if self.verbose:
            logger.info(f"yes24에서 {len(links)}개의 상세 페이지 링크를 찾았습니다")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        """
        yes24 상세 페이지에서 책 정보 추출

        Args:
            soup: BeautifulSoup 객체

        Returns:
            책 정보 딕셔너리
        """
        book_info = {
            'title': '',
            'author': '',
            'category': '',
            'publisher': '',
            'isbn': ''
        }

        try:
            # 책 제목 추출
            title_elem = soup.find('h2', class_='gd_name')
            if title_elem:
                book_info['title'] = title_elem.get_text(strip=True)

            # 저자 정보 추출
            author_elem = soup.find('span', class_='gd_auth')
            if author_elem:
                book_info['author'] = author_elem.get_text(strip=True)

            # 출판사 정보 추출
            publisher_elem = soup.find('span', class_='gd_pub')
            if publisher_elem:
                book_info['publisher'] = publisher_elem.get_text(strip=True)

            # 카테고리 정보 추출 (실제 구조에 맞게 수정)
            category_text = self._extract_yes24_category(soup)
            if category_text:
                book_info['category'] = category_text

            # ISBN 추출 (실제 구조에 맞게 수정)
            isbn_text = self._extract_yes24_isbn(soup)
            if isbn_text:
                book_info['isbn'] = isbn_text

        except Exception as e:
            logger.error(f"책 정보 추출 중 오류: {e}")

        return book_info

    def _extract_yes24_category(self, soup: BeautifulSoup) -> str:
        """
        yes24 상세 페이지에서 카테고리 정보 추출

        Args:
            soup: BeautifulSoup 객체

        Returns:
            카테고리 문자열
        """
        try:
            # "관련분류" 텍스트를 포함하는 요소 찾기
            related_category_elements = soup.find_all(string=lambda text: text and '관련분류' in text)

            for elem in related_category_elements:
                parent = elem.parent
                if parent:
                    # 관련분류 섹션에서 카테고리 링크들 찾기
                    category_links = parent.find_all('a', href=lambda href: href and '/product/category/display/' in href)

                    if category_links:
                        # 카테고리 경로 구성
                        category_path = []
                        for link in category_links:
                            category_text = link.get_text(strip=True)
                            if category_text and category_text not in category_path:
                                category_path.append(category_text)

                        if category_path:
                            return ' > '.join(category_path)

            return ''

        except Exception as e:
            logger.error(f"카테고리 추출 중 오류: {e}")
            return ''

    def _extract_yes24_isbn(self, soup: BeautifulSoup) -> str:
        """
        yes24 상세 페이지에서 ISBN 정보 추출

        Args:
            soup: BeautifulSoup 객체

        Returns:
            ISBN 문자열
        """
        try:
            # ISBN 패턴을 포함하는 텍스트 찾기
            isbn_patterns = [
                r'ISBN13\s*(\d{13})',
                r'ISBN10\s*(\d{10})',
                r'ISBN\s*(\d{10,13})',
                r'(\d{10,13})'
            ]

            # 전체 페이지 텍스트에서 ISBN 패턴 검색
            page_text = soup.get_text()

            for pattern in isbn_patterns:
                import re
                match = re.search(pattern, page_text)
                if match:
                    return match.group(1)

            return ''

        except Exception as e:
            logger.error(f"ISBN 추출 중 오류: {e}")
            return ''

# AladinBookstore stub
class AladinBookstore(AbstractBookstore):
    """알라딘 서점 검색 구현 스텁"""
    BASE_URL = 'https://www.aladin.co.kr'

    def build_search_url(self, keyword: str) -> str:
        """알라딘 검색 URL을 생성합니다."""
        encoded_keyword = quote(keyword)
        return f"{self.BASE_URL}/search/wsearchresult.aspx?SearchTarget=All&SearchWord={encoded_keyword}"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        """알라딘 검색 결과에서 상세 페이지 링크를 추출합니다."""
        links = []
        seen_urls = set()
        search_result_div = soup.find('div', id='Search3_Result')
        if search_result_div:
            # 상세 페이지 링크는 /shop/wproduct.aspx?ItemId=... 형태
            for a_tag in search_result_div.find_all('a', href=re.compile(r'/shop/wproduct\.aspx\?ItemId=\d+')):
                if len(links) >= self.MAX_RESULTS:
                    break

                href = a_tag.get('href')
                # 리뷰 링크는 제외하고, 고유한 URL만 추가
                if href and '_CommentReview' not in href:
                    full_url = urljoin(self.BASE_URL, href)
                    if full_url not in seen_urls:
                        links.append(full_url)
                        seen_urls.add(full_url)

        if self.verbose:
            logger.info(f"알라딘에서 {len(links)}개의 상세 페이지 링크를 찾았습니다.")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        """알라딘 상세 페이지에서 책 정보를 추출합니다."""
        info = {'title': '', 'author': '', 'category': ''}

        # 1. <title> 태그에서 제목, 저자 추출
        title_tag = soup.find('title')
        if title_tag and title_tag.string:
            parts = [p.strip() for p in title_tag.string.split('|')]
            if len(parts) >= 3:
                info['author'] = parts[-2]
                info['title'] = ' | '.join(parts[:-2])

        # 1.5 메타 태그에서 저자 추출 (name 또는 og:author)
        meta_author = soup.find('meta', attrs={'name': 'author'}) or soup.find('meta', attrs={'property': 'og:author'})
        if meta_author and meta_author.get('content'):
            info['author'] = meta_author['content']

        # 1.6 메타 태그에서 제목 추출 (og:title)
        meta_title = soup.find('meta', attrs={'property': 'og:title'}) or soup.find('meta', attrs={'name': 'title'})
        if meta_title and meta_title.get('content'):
            info['title'] = meta_title['content']

        # 2. ul id="ulCategory"에서 카테고리 추출
        category_ul = soup.find('ul', id='ulCategory')
        if category_ul:
            category_links = category_ul.find_all('a')
            if category_links:
                category_parts = [link.get_text(strip=True) for link in category_links]
                # 첫 번째 링크는 "국내도서" 같은 최상위 카테고리이므로 필요시 포함/제외
                info['category'] = ' > '.join(category_parts)

        return info

# RidibooksBookstore implementation
class RidibooksBookstore(AbstractBookstore):
    BASE_URL = 'https://ridibooks.com'

    def build_search_url(self, keyword: str) -> str:
        encoded = quote(keyword)
        return f"{self.BASE_URL}/search?q={encoded}&adult_exclude=n"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        links: List[str] = []
        seen = set()
        # ul > li > div > div > a
        for a_tag in soup.select("ul > li > div > div > a"):
            href = a_tag.get('href', '')
            # strip query params to match pure book path
            path = href.split('?', 1)[0]
            if re.match(r'^/books/\d+$', path):
                full = urljoin(self.BASE_URL, path)
                if full not in seen:
                    seen.add(full)
                    links.append(full)
        # fallback: find any book links if primary selector yields none
        if not links:
            for a_tag in soup.find_all('a', href=True):
                href = a_tag['href']
                path = href.split('?', 1)[0]
                if re.match(r'^/books/\d+$', path):
                    full = urljoin(self.BASE_URL, path)
                    if full not in seen:
                        seen.add(full)
                        links.append(full)
        if self.verbose:
            logger.info(f"Ridibooks에서 {len(links)}개의 상세 페이지 링크를 찾았습니다")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        info = {'title': '', 'author': '', 'category': ''}
        # 제목 추출: og:title 메타 태그 우선, 실패 시 <h1> 태그 사용
        meta = soup.find('meta', property='og:title') or soup.find('meta', attrs={'name': 'title'})
        if meta and meta.get('content'):
            info['title'] = meta['content'].strip()
        else:
            h1 = soup.select_one('h1')
            if h1 and h1.get_text(strip=True):
                info['title'] = h1.get_text(strip=True)
        # 저자 추출: 디테일 페이지 author 섹션의 링크
        author_elem = soup.select_one("div.rigrid-bec17 a[href^='/author/']")
        if author_elem:
            info['author'] = author_elem.get_text(strip=True)
        else:
            # fallback1: any /author/<digits> 링크
            elem = soup.find('a', href=re.compile(r'^/author/\d+'))
            if elem:
                info['author'] = elem.get_text(strip=True)
        # fallback2: specific author list under header section
        if not info['author']:
            list_elem = soup.select_one("ul.rigrid-15fcnk6 a[href^='/author/']")
            if list_elem:
                info['author'] = list_elem.get_text(strip=True)
        # 카테고리 추출: /category/숫자열 링크 텍스트
        categories = []
        selector = "#books_contents section.detail_body ul li a[href^='/category/']"
        for link in soup.select(selector):
            text = link.get_text(strip=True)
            if text:
                categories.append(text)
        if categories:
            info['category'] = ' > '.join(categories)
        return info

class NaverShoppingBookstore(AbstractBookstore):
    BASE_URL = 'https://search.shopping.naver.com'

    def build_search_url(self, keyword: str) -> str:
        encoded = quote(keyword)
        return f"{self.BASE_URL}/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&query={encoded}&sort=REL"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        """네이버쇼핑 검색 결과에서 상세 페이지 링크 리스트를 반환합니다."""
        links: List[str] = []
        # 모든 <script> 태그에서 JSON 텍스트 검색
        raw = None
        for s in soup.find_all('script'):
            text = s.get_text() or ''
            if text.strip().startswith('{') and 'SearchAll' in text:
                raw = text
                break
        if not raw:
            if self.verbose:
                logger.warning("네이버쇼핑 JSON 스크립트를 찾을 수 없습니다")
            return links
        # JSON 파싱
        try:
            data = json.loads(raw)
        except json.JSONDecodeError as e:
            if self.verbose:
                logger.error(f"네이버쇼핑 JSON 파싱 실패: {e}")
            return links
        # React Query state에서 SearchAll 아이템 추출
        queries = data.get('props', {}).get('pageProps', {}).get('dehydratedState', {}).get('queries', [])
        seen = set()
        for q in queries:
            key = q.get('queryKey')
            if isinstance(key, list) and key and key[0] == 'SearchAll':
                items = q.get('state', {}).get('data', {}).get('SearchAll', {}).get('bookSasResult', {}).get('itemList', [])
                if not isinstance(items, list):
                    break
                for item in items:
                    item_id = item.get('id')
                    if item_id and item_id not in seen:
                        seen.add(item_id)
                        links.append(f"{self.BASE_URL}/book/catalog/{item_id}")
                break
        if self.verbose:
            logger.info(f"네이버쇼핑에서 {len(links)}개의 상세 페이지 링크를 찾았습니다")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        """
        네이버쇼핑 상세 페이지에서 책 정보를 추출합니다.
        """
        info: Dict[str, str] = {'title': '', 'author': '', 'category': ''}
        try:
            # 제목
            title_elem = soup.find('div', class_='bookTitle_book_name__')
            if title_elem:
                info['title'] = title_elem.get_text(strip=True)
            # 저자
            author_elem = soup.find('div', class_='bookTitle_info_content__')
            if author_elem:
                info['author'] = author_elem.get_text(strip=True)
            # 카테고리
            category_elem = soup.find('div', class_='bookCatalogTop_breadcrumb__')
            if category_elem:
                parts = [p.strip() for p in category_elem.get_text().split('>')]
                info['category'] = ' > '.join(parts)
        except Exception as e:
            logger.error(f"네이버쇼핑 상세 정보 추출 중 오류: {e}")
        return info

    def search_by_keyword(self, keyword: str) -> List[Tuple[str, str, str, str, str]]:
        """
        네이버쇼핑 API 를 사용하여 키워드 검색 후 상세 페이지 메타데이터를 반환합니다.
        """
        encoded = quote(keyword)
        search_url = (
            f"{self.BASE_URL}/api/search?query={encoded}"
            "&entities=SEARCH_PAGING"
            "&pagingIndex=1&pagingSize=40&sort=REL&bookTabType=ALL"
        )
        resp = self.session.get(search_url, timeout=10, verify=False)
        try:
            data = resp.json()
        except Exception:
            if self.verbose:
                logger.error("네이버쇼핑 API 응답을 JSON으로 파싱하지 못했습니다")
            return []
        # API 응답 구조에서 item list 접근
        items = data.get('searchResult', {}).get('items', [])
        results: List[Tuple[str, str, str, str, str]] = []
        for item in items[:self.MAX_RESULTS]:
            item_id = item.get('id')
            if not item_id:
                continue
            detail_url = f"{self.BASE_URL}/book/catalog/{item_id}"
            # 상세 페이지 요청
            resp2 = self.session.get(detail_url, timeout=10, verify=False)
            resp2.encoding = 'utf-8'
            soup = BeautifulSoup(resp2.text, 'html.parser')
            info = self.extract_book_info(soup)
            results.append((info.get('title',''), info.get('author',''), info.get('category',''), detail_url, search_url))
        return results

class MunpiaBookstore(AbstractBookstore):
    BASE_URL = 'https://novel.munpia.com'

    def build_search_url(self, keyword: str) -> str:
        """문피아 검색 URL을 구성합니다 (공백은 %20으로 인코딩)."""
        encoded = quote(keyword, safe='')
        return f"{self.BASE_URL}/page/hd.platinum/view/search/keyword/{encoded}/order/search_result"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        """문피아 검색 결과 페이지에서 상세 페이지 링크를 추출합니다."""
        links: List[str] = []
        seen_urls = set()
        selector = (
            "div#SEARCH-BOX.section2 > div.ebook_lists > div.article_wrap >"
            " div.article > dl.detail > dt > a"
        )
        for a_tag in soup.select(selector):
            href = a_tag.get('href')
            if not href:
                continue
            full_url = urljoin(self.BASE_URL, href) if href.startswith('/') else href
            if full_url not in seen_urls:
                seen_urls.add(full_url)
                links.append(full_url)
        if self.verbose:
            logger.info(f"문피아에서 {len(links)}개의 상세 페이지 링크를 찾았습니다")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        """문피아 상세 페이지에서 책 정보를 추출합니다."""
        book_info = {'title': '', 'author': '', 'category': ''}
        # 제목 추출: og:title 또는 <meta name="title">
        meta_title = soup.find('meta', property='og:title') or soup.find('meta', attrs={'name': 'title'})
        if meta_title and meta_title.get('content'):
            book_info['title'] = meta_title['content'].strip()
        # 저자 추출: og:description에서 '저자명 - 설명' 형태
        meta_desc = soup.find('meta', property='og:description') or soup.find('meta', attrs={'name': 'description'})
        if meta_desc and meta_desc.get('content'):
            content = meta_desc['content'].strip()
            # '저자명 - 설명'에서 저자명만 추출
            book_info['author'] = content.split(' - ')[0]
        else:
            # fallback: 작가 링크
            author_elem = soup.select_one('a[href^="/writer/"]')
            if author_elem:
                book_info['author'] = author_elem.get_text(strip=True)
        # 카테고리 추출: p.meta-path
        meta_path_elem = soup.select_one('p.meta-path')
        if meta_path_elem:
            path_text = meta_path_elem.get_text(strip=True)
            book_info['category'] = ' > '.join([p.strip() for p in path_text.split('>')])
        return book_info

# NaverSeriesBookstore 구현
class NaverSeriesBookstore(AbstractBookstore):
    BASE_URL = 'https://series.naver.com'

    def build_search_url(self, keyword: str) -> str:
        encoded = quote(keyword)
        return f"{self.BASE_URL}/search/search.series?t=all&fs=novel&q={encoded}"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        links: List[str] = []
        seen = set()
        # 검색 결과 리스트에서 링크 추출
        for a_tag in soup.select('ul.lst_list li a[class="N=a:nov.title"]'):
            href = a_tag.get('href')
            if href:
                full = urljoin(self.BASE_URL, href)
                if full not in seen:
                    seen.add(full)
                    links.append(full)
        if self.verbose:
            logger.info(f"NaverSeries에서 {len(links)}개의 상세 페이지 링크를 찾았습니다")
        return links

    def extract_book_info(self, soup: BeautifulSoup) -> Dict[str, str]:
        info: Dict[str, str] = {'title': '', 'author': '', 'category': ''}
        # 제목 추출
        if soup.title and soup.title.string:
            info['title'] = soup.title.string.strip()
        # 저자 추출: 작가 정보 컨테이너 내 <strong> 태그 활용
        author_container = soup.find('div', id='_otherProductByPerson')
        if author_container:
            strongs = author_container.find_all('strong')
            if len(strongs) >= 2:
                info['author'] = strongs[1].get_text(strip=True)
        # author가 비어있을 경우 meta name='description' 우선 Fallback, og:description 다음 사용
        if not info['author']:
            # 1) meta name='description'
            meta_name_desc = soup.find('meta', attrs={'name': 'description'})
            if meta_name_desc and meta_name_desc.get('content'):
                desc = meta_name_desc['content']
                m = re.search(r'작가[:：]\s*([^,]+)', desc)
                if m:
                    info['author'] = m.group(1).strip()
            # 2) meta property='og:description'
        if not info['author']:
            meta_og = soup.find('meta', attrs={'property': 'og:description'})
            if meta_og and meta_og.get('content'):
                desc = meta_og['content']
                m = re.search(r'작가[:：]\s*([^,」]+)', desc)
                if m:
                    info['author'] = m.group(1).strip()
        # 카테고리 추출
        category_elem = soup.select_one('div#content > ul.end_info > li.info_lst > ul > li span a')
        if category_elem:
            info['category'] = category_elem.get_text(strip=True)
        return info

# 기본 Bookstore alias
DefaultBookstore = Bookstore
# 예스24 구현 클래스 alias
Yes24Bookstore = Bookstore

# 공개 API
__all__ = [
    'AbstractBookstore',
    'Bookstore',
    'Yes24Bookstore',
    'DefaultBookstore',
    'AladinBookstore',
    'RidibooksBookstore',
    'NaverShoppingBookstore',
    'MunpiaBookstore',
    'NaverSeriesBookstore'
]


import {useEffect, useState} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import { Button, Tabs, Tab, Spinner, Card, ButtonGroup } from 'react-bootstrap';
import {rawJsonGetReq} from './Common';

// 카테고리에서 최하위 + 바로 상위 두 단계를 추출 (공백으로 연결)
// 예: "소설/시/희곡 > SF > 한국SF" → "SF 한국SF"
// 예: "소설/시/희곡 > 중국소설" → "소설/시/희곡 중국소설"
// 예: "한국SF" → "한국SF"
const getTwoLevelCategory = (category) => {
  if (!category) return '';
  const parts = category.split('>').map(s => s.trim()).filter(Boolean);
  if (parts.length <= 1) return parts[0] || '';
  return `${parts[parts.length - 2]} ${parts[parts.length - 1]}`;
};

// 서점 탭 정의 (supportsIsbn: ISBN 검색 지원 여부)
const STORES = [
  { key: 'yes24', label: 'Yes24', supportsIsbn: true },
  { key: 'aladin', label: '알라딘', supportsIsbn: true },
  { key: 'ridi', label: 'RIDI', supportsIsbn: false },
  { key: 'munpia', label: '문피아', supportsIsbn: false },
  { key: 'naverseries', label: '시리즈', supportsIsbn: false }
];

export default function Bookstore(props) {
  const [title, setTitle] = useState('');
  const [author, setAuthor] = useState('');
  const [isbn, setIsbn] = useState('');
  const [activeKey, setActiveKey] = useState(STORES[0].key);
  const [data, setData] = useState({});

  // bookInfo 변경 시 로컬 필드만 동기화 (검색은 트리거하지 않음)
  useEffect(() => {
    setTitle(props.bookInfo.title || '');
    setAuthor(props.bookInfo.author || '');
    setIsbn(props.bookInfo.isbn || '');
  }, [props.bookInfo]);

  // 책 정보 로딩 또는 이름 변경 시에만 자동 검색 실행
  useEffect(() => {
    if (!props.searchTrigger) return; // 초기 마운트 시 스킵

    // 탭 및 데이터 초기화
    setData({});
    setActiveKey(STORES[0].key);
    if (props.onCategoriesFound) {
      props.onCategoriesFound({});
    }

    // Yes24, 알라딘 자동 검색: ISBN → 저자+제목 → 제목 순으로 시도
    const autoSearch = async (store) => {
      const currentIsbn = props.bookInfo.isbn || '';
      const currentTitle = props.bookInfo.title || '';
      const currentAuthor = props.bookInfo.author || '';

      if (!currentIsbn && !currentTitle && !currentAuthor) return null;

      // 1. ISBN 검색 시도 (ISBN이 있는 경우)
      if (currentIsbn) {
        const result = await fetchWithMethodInternal(store, 'isbn', currentIsbn, currentTitle, currentAuthor);
        if (result?.status === 'success' && result?.result?.length > 0) {
          return result;
        }
      }

      // 2. 저자+제목 검색 시도
      if (currentTitle || currentAuthor) {
        const result = await fetchWithMethodInternal(store, 'title_author', currentIsbn, currentTitle, currentAuthor);
        if (result?.status === 'success' && result?.result?.length > 0) {
          return result;
        }
      }

      // 3. 제목만으로 검색 시도 (저자+제목으로 결과가 없는 경우)
      if (currentTitle) {
        return await fetchWithMethodInternal(store, 'title_only', currentIsbn, currentTitle, currentAuthor);
      }

      return null;
    };

    const runAutoSearch = async () => {
      const yes24Result = await autoSearch('yes24');
      const aladinResult = await autoSearch('aladin');

      // 두 서점 검색 결과의 모든 카테고리(마지막 레벨만)를 수집하여 부모에게 전달
      if (props.onCategoriesFound) {
        const categories = {};
        if (yes24Result?.status === 'success' && yes24Result?.result?.length > 0) {
          yes24Result.result.forEach((item, idx) => {
            const deepest = getTwoLevelCategory(item.category);
            if (deepest) {
              categories[`yes24_${idx}`] = deepest;
            }
          });
        }
        if (aladinResult?.status === 'success' && aladinResult?.result?.length > 0) {
          aladinResult.result.forEach((item, idx) => {
            const deepest = getTwoLevelCategory(item.category);
            if (deepest) {
              categories[`aladin_${idx}`] = deepest;
            }
          });
        }
        props.onCategoriesFound(categories);
      }
    };

    runAutoSearch();
  }, [props.searchTrigger]);

  // 내부 검색 함수 (자동 검색용, 결과 반환)
  const fetchWithMethodInternal = (store, method, isbnVal, titleVal, authorVal) => {
    return new Promise((resolve) => {
      setData(prev => ({ ...prev, [store]: { loading: true } }));

      const params = new URLSearchParams();

      switch (method) {
        case 'isbn':
          if (isbnVal) params.append('isbn', isbnVal);
          break;
        case 'title_author':
          if (titleVal) params.append('title', titleVal);
          if (authorVal) params.append('author', authorVal);
          break;
        case 'title_only':
          if (titleVal) params.append('title', titleVal);
          break;
        default:
          if (isbnVal) params.append('isbn', isbnVal);
          if (titleVal) params.append('title', titleVal);
          if (authorVal) params.append('author', authorVal);
      }

      if (params.toString() === '') {
        setData(prev => ({ ...prev, [store]: { error: true, message: '검색어가 없습니다.' } }));
        resolve(null);
        return;
      }

      rawJsonGetReq(
        `/search/bookstore/${store}?${params.toString()}`,
        (json) => {
          setData(prev => ({ ...prev, [store]: json }));

          resolve(json);
        },
        (error) => {
          console.error(error);
          setData(prev => ({ ...prev, [store]: { error: true, message: '검색 중 오류가 발생했습니다.' } }));
          resolve(null);
        }
      );
    });
  };

  // 특정 검색 방법으로 검색 수행 (버튼 클릭용)
  const fetchWithMethod = (store, method) => {
    const storeInfo = STORES.find(s => s.key === store);

    // 검색어 결정
    let searchTerms = '';
    switch (method) {
      case 'isbn':
        searchTerms = isbn;
        break;
      case 'title_author':
        searchTerms = `${title}_${author}`;
        break;
      default:
        searchTerms = `${isbn}_${title}_${author}`;
    }

    // 캐시 키에 검색어 포함
    const cacheKey = `${store}_${method}_${searchTerms}`;

    // 이미 해당 검색 결과가 있으면 재사용
    if (data[cacheKey] && !data[cacheKey].loading && !data[cacheKey].error) {
      setData(prev => ({ ...prev, [store]: data[cacheKey] }));
      return;
    }

    // ISBN 미지원 서점에서 ISBN 검색 시도 시 에러 표시
    if (method === 'isbn' && !storeInfo?.supportsIsbn) {
      setData(prev => ({ ...prev, [store]: { error: true, message: '이 서점은 ISBN 검색을 지원하지 않습니다.' } }));
      return;
    }

    // 새 검색 시작 시 기존 결과 초기화
    setData(prev => ({ ...prev, [store]: { loading: true } }));

    const params = new URLSearchParams();

    switch (method) {
      case 'isbn':
        if (isbn) params.append('isbn', isbn);
        break;
      case 'title_author':
        if (title) params.append('title', title);
        if (author) params.append('author', author);
        break;
      default:
        if (isbn) params.append('isbn', isbn);
        if (title) params.append('title', title);
        if (author) params.append('author', author);
    }

    if (params.toString() === '') {
      setData(prev => ({ ...prev, [store]: { error: true, message: '검색어가 없습니다.' } }));
      return;
    }

    rawJsonGetReq(
      `/search/bookstore/${store}?${params.toString()}`,
      (json) => {
        // 결과를 store와 cacheKey 둘 다에 저장
        setData(prev => {
          const newData = { ...prev, [store]: json, [cacheKey]: json };

          // Yes24 또는 알라딘 검색 결과의 모든 카테고리(마지막 레벨만)를 부모에게 전달
          if ((store === 'yes24' || store === 'aladin') && props.onCategoriesFound) {
            const categories = {};
            // yes24 카테고리
            const yes24Data = store === 'yes24' ? json : newData['yes24'];
            if (yes24Data?.status === 'success' && yes24Data?.result?.length > 0) {
              yes24Data.result.forEach((item, idx) => {
                const deepest = getTwoLevelCategory(item.category);
                if (deepest) {
                  categories[`yes24_${idx}`] = deepest;
                }
              });
            }
            // aladin 카테고리
            const aladinData = store === 'aladin' ? json : newData['aladin'];
            if (aladinData?.status === 'success' && aladinData?.result?.length > 0) {
              aladinData.result.forEach((item, idx) => {
                const deepest = getTwoLevelCategory(item.category);
                if (deepest) {
                  categories[`aladin_${idx}`] = deepest;
                }
              });
            }
            props.onCategoriesFound(categories);
          }

          return newData;
        });
      },
      (error) => {
        console.error(error);
        setData(prev => ({ ...prev, [store]: { error: true, message: '검색 중 오류가 발생했습니다.' } }));
      }
    );
  };

  // 탭 내용 렌더링 함수
  const renderTabContent = (storeKey) => {
    const result = data[storeKey];
    const storeInfo = STORES.find(s => s.key === storeKey);

    return (
      <div>
        {/* 검색 버튼들 */}
        <div className="p-2 border-bottom">
          <ButtonGroup>
            <Button
              variant={isbn && storeInfo?.supportsIsbn ? "outline-primary" : "outline-secondary"}
              className="btn-xs"
              onClick={() => fetchWithMethod(storeKey, 'isbn')}
              disabled={result?.loading || !isbn || !storeInfo?.supportsIsbn}
              title={!isbn ? "ISBN 정보 없음" : (!storeInfo?.supportsIsbn ? "이 서점은 ISBN 검색 미지원" : "")}
            >
              ISBN
            </Button>
            <Button
              variant={(title || author) ? "outline-primary" : "outline-secondary"}
              className="btn-xs"
              onClick={() => fetchWithMethod(storeKey, 'title_author')}
              disabled={result?.loading || (!title && !author)}
            >
              저자+제목
            </Button>
          </ButtonGroup>
          {result?.search_url && (
            <a href={result.search_url} target="_blank" rel="noreferrer" className="ms-2">
              <Button variant="outline-secondary" className="btn-xs">서점에서 보기</Button>
            </a>
          )}
        </div>

        {/* 검색 결과 */}
        {result?.loading && (
          <div className="text-center p-2"><Spinner animation="border" size="sm" /></div>
        )}

        {result?.error && (
          <div className="text-danger p-2">{result.message || '검색 중 오류가 발생했습니다.'}</div>
        )}

        {result && !result.loading && !result.error && (
          <>
            {result.status === 'success' && result.result.length > 0 ? (
              result.result.map((item, idx) => (
                <div key={item.book_url || idx} className="p-1 border-bottom">
                  <div>
                    <a href={item.book_url} target="_blank" rel="noreferrer">
                      <strong>{item.title}</strong>
                    </a>
                  </div>
                  <small className="text-muted">
                    {item.author && <span>{item.author}</span>}
                    {item.category && <span> | {item.category}</span>}
                    {item.isbn && <span> | ISBN: {item.isbn}</span>}
                  </small>
                </div>
              ))
            ) : (
              <p className="p-2 text-muted mb-0">검색 결과가 없습니다.</p>
            )}
          </>
        )}
      </div>
    );
  };

  return (
    <Card>
        <Card.Header>
            서점 검색
        </Card.Header>

        <Card.Body className="p-0">
            <Tabs
              activeKey={activeKey}
              onSelect={(k) => setActiveKey(k)}
              variant="tabs"
              className="m-0">
            {STORES.map((store) => (
                <Tab eventKey={store.key} title={store.label} key={store.key}>
                  {renderTabContent(store.key)}
                </Tab>
            ))}
            </Tabs>
        </Card.Body>
    </Card>
  );
}

Bookstore.propTypes = {
  bookInfo: PropTypes.shape({
    author: PropTypes.string,
    title: PropTypes.string,
    isbn: PropTypes.string
  }).isRequired,
  searchTrigger: PropTypes.number,
  onCategoriesFound: PropTypes.func
};

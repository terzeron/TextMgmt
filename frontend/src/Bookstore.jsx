import {useEffect, useState} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import { Button, Tabs, Tab, Spinner, Card, ButtonGroup } from 'react-bootstrap';
import {getApiUrlPrefix} from './Common';

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

  useEffect(() => {
    // bookInfo 변경 사항 반영 및 탭 초기화
    setTitle(props.bookInfo.title || '');
    setAuthor(props.bookInfo.author || '');
    setIsbn(props.bookInfo.isbn || '');
    setData({});
    setActiveKey(STORES[0].key);
  }, [props.bookInfo]);

  // 특정 검색 방법으로 검색 수행
  const fetchWithMethod = async (store, method) => {
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

    try {
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

      const res = await fetch(`${getApiUrlPrefix()}/search/bookstore/${store}?${params.toString()}`);
      const json = await res.json();
      // 결과를 store와 cacheKey 둘 다에 저장
      setData(prev => ({ ...prev, [store]: json, [cacheKey]: json }));
    } catch (e) {
      console.error(e);
      setData(prev => ({ ...prev, [store]: { error: true, message: '검색 중 오류가 발생했습니다.' } }));
    }
  };

  // 탭 내용 렌더링 함수
  const renderTabContent = (storeKey) => {
    const result = data[storeKey];
    const storeInfo = STORES.find(s => s.key === storeKey);

    return (
      <div>
        {/* 검색 버튼들 */}
        <div className="p-2 border-bottom">
          <ButtonGroup size="sm">
            <Button
              variant={isbn && storeInfo?.supportsIsbn ? "outline-primary" : "outline-secondary"}
              onClick={() => fetchWithMethod(storeKey, 'isbn')}
              disabled={result?.loading || !isbn || !storeInfo?.supportsIsbn}
              title={!isbn ? "ISBN 정보 없음" : (!storeInfo?.supportsIsbn ? "이 서점은 ISBN 검색 미지원" : "")}
            >
              ISBN
            </Button>
            <Button
              variant={(title || author) ? "outline-primary" : "outline-secondary"}
              onClick={() => fetchWithMethod(storeKey, 'title_author')}
              disabled={result?.loading || (!title && !author)}
            >
              저자+제목
            </Button>
          </ButtonGroup>
          {result?.search_url && (
            <a href={result.search_url} target="_blank" rel="noreferrer" className="ms-2">
              <Button variant="outline-secondary" size="sm">서점에서 보기</Button>
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
              className="m-0"
            >
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
  }).isRequired
};

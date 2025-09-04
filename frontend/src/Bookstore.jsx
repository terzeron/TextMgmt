import {useEffect, useState} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import { Button, Tabs, Tab, Spinner, Card } from 'react-bootstrap';
// 서점 탭 정의
const STORES = [
  { key: 'yes24', label: 'Yes24' },
  { key: 'aladin', label: '알라딘' },
  { key: 'munpia', label: '문피아' },
  { key: 'ridi', label: 'RIDI' },
  { key: 'naverseries', label: '시리즈' }
];

export default function Bookstore(props) {
  const [keyword, setKeyword] = useState('');
  const [activeKey, setActiveKey] = useState(STORES[0].key);
  const [data, setData] = useState({});

  useEffect(() => {
    // bookInfo 변경 사항 반영 및 탭 초기화
    const extractedWords = (props.bookInfo.author + ' ' + props.bookInfo.title).match(/[\uAC00-\uD7A3a-zA-Z0-9]+/g);
    const kw = extractedWords ? extractedWords.join(' ') : '';
    setKeyword(kw);
    setData({});
    setActiveKey(STORES[0].key);
  }, [props.bookInfo]);

  // 선택된 탭에 대해 동적으로 검색 수행
  const fetchStoreData = async (store) => {
    if (!keyword) return;
    setData(prev => ({ ...prev, [store]: { loading: true } }));
    try {
      const res = await fetch(`/search/bookstore/${store}?title=${encodeURIComponent(keyword)}`);
      const json = await res.json();
      setData(prev => ({ ...prev, [store]: json }));
    } catch (e) {
      console.error(e);
      setData(prev => ({ ...prev, [store]: { error: true } }));
    }
  };

  // 탭 내용 렌더링 함수
  const renderTabContent = (storeKey) => {
    const result = data[storeKey]
    if (!result) {
      return null
    }
    if (result.loading) {
      return <div className="text-center p-1"><Spinner animation="border" /></div>
    }
    // 검색어와 서점 검색 결과 링크는 항상 노출
    return (
      <div>
        <div className="p-1 border">
            <span><strong>검색어:</strong> </span>
            <span>
            <a href={result.search_url} target="_blank" rel="noreferrer">
                <Button variant="outline-primary" size="sm">&quot;{result.search_keyword}&quot; 검색</Button>
            </a>
            </span>
        </div>

        {result.status === 'success' && result.result.length > 0 ? (
          result.result.map(item => (
            <div key={item.book_url} className="p-1 border">
              <div><strong>도서명:</strong> <a href={item.book_url} target="_blank" rel="noreferrer">
                  <Button variant="outline-primary" size="sm">&quot;{item.title}&quot;</Button>
                </a></div>
              <div><strong>저자명:</strong> {item.author}</div>
              <div><strong>카테고리:</strong> {item.category}</div>
            </div>
          ))
        ) : (
          <p>검색 결과가 없습니다.</p>
        )}
      </div>
    )
  }

  return (
    <Card>
        <Card.Header>
            서점 검색 결과
        </Card.Header>

        <Card.Body>
            <Tabs
            activeKey={activeKey}
            onSelect={(k) => { setActiveKey(k); fetchStoreData(k); }}
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
    title: PropTypes.string
  }).isRequired
};
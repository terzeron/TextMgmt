import {useState, useEffect, useRef} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Card, Badge, Table} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {jsonPostReq} from './Common';

// 카테고리 키(예: "aladin_0_0")에서 서점명 추출
const getStoreName = (key) => {
    const idx = key.indexOf('_');
    return idx > 0 ? key.substring(0, idx) : key;
};

const STORE_COLORS = {
    yes24: '#6A1B9A',
    aladin: '#0D47A1',
    ridi: '#00897B',
};

export default function SimilarityDebug({suggestedCategories, categoryList, showWhenEmpty = false}) {
    const [isOpen, setIsOpen] = useState(false);
    const [debugInfo, setDebugInfo] = useState(null);
    const requestRef = useRef(0);

    // 디버그 정보는 backend(store_category_matcher)가 계산한다. 늦게 도착한 이전 요청의 응답은 버린다.
    useEffect(() => {
        const requestId = ++requestRef.current;
        setDebugInfo(null);
        if (!suggestedCategories || Object.keys(suggestedCategories).length === 0 || !categoryList?.length) {
            return;
        }
        jsonPostReq(
            '/bookstore/category-suggestions?content_type=book',
            {suggested: suggestedCategories, categories: categoryList, top_n: 10, debug: true},
            (result) => {
                if (requestId === requestRef.current) {
                    setDebugInfo(result?.debug || null);
                }
            },
            () => {},
        );
    }, [suggestedCategories, categoryList]);

    if (!debugInfo) {
        if (!showWhenEmpty) {
            return null;
        }
        // 만화는 웹툰 서점과 일치할 때만 카테고리가 생긴다. 패널을 숨기지 않고 이유를 알린다.
        return (
            <Card>
                <Card.Header
                    onClick={() => setIsOpen(!isOpen)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                    유사도 계산 디버그
                </Card.Header>
                {isOpen && (
                    <Card.Body className="p-2 text-muted" style={{fontSize: '0.8rem'}}>
                        서점 검색 결과에서 추출된 카테고리가 없습니다. 서점을 수동으로 검색하면 다시 계산합니다.
                    </Card.Body>
                )}
            </Card>
        );
    }

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(!isOpen)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                유사도 계산 디버그
            </Card.Header>

            {isOpen && (
                <Card.Body className="p-2" style={{fontSize: '0.8rem'}}>
                    {/* 서점별 추출된 키워드 */}
                    <div className="mb-3">
                        <strong>서점 카테고리에서 추출된 키워드:</strong>
                        <Table size="sm" bordered className="mt-1 mb-0">
                            <thead>
                                <tr>
                                    <th style={{width: '60px'}}>서점</th>
                                    <th>원본 카테고리</th>
                                    <th>추출 키워드</th>
                                </tr>
                            </thead>
                            <tbody>
                                {Object.entries(debugInfo.bookstoreKeywords || {}).map(([store, info]) => {
                                    const name = getStoreName(store);
                                    return (
                                    <tr key={store}>
                                        <td>
                                            <Badge
                                                style={{
                                                    backgroundColor: STORE_COLORS[name] || '#455A64',
                                                    minWidth: '55px'
                                                }}
                                            >
                                                {name}
                                            </Badge>
                                        </td>
                                        <td className="text-muted" style={{fontSize: '0.75rem'}}>
                                            {info?.original || ''}
                                        </td>
                                        <td>
                                            {(info?.keywords || []).map(kw => (
                                                <Badge key={kw} bg="primary" className="me-1">{kw}</Badge>
                                            ))}
                                        </td>
                                    </tr>
                                    );
                                })}
                            </tbody>
                        </Table>
                    </div>

                    {/* 상위 카테고리별 유사도 계산 과정 */}
                    <div>
                        <strong>상위 {debugInfo.categoryDetails.length}개 카테고리 유사도:</strong>
                        <Table size="sm" bordered striped className="mt-1 mb-0">
                            <thead>
                                <tr>
                                    <th style={{width: '30px'}}>#</th>
                                    <th>디렉토리</th>
                                    <th>매칭 상세</th>
                                    <th>총점</th>
                                </tr>
                            </thead>
                            <tbody>
                                {debugInfo.categoryDetails.map((detail, idx) => {
                                    // 1위: 현재 색상, 2-5위: 옅은 노란색
                                    const rowStyle = idx === 0 ? {} : (idx < 5 ? {backgroundColor: '#FFF9C4'} : {});
                                    return (
                                    <tr key={detail.category} style={rowStyle}>
                                        <td className="text-center">{idx + 1}</td>
                                        <td>
                                            <div><strong>{detail.category}</strong></div>
                                            <div className="text-muted" style={{fontSize: '0.7rem'}}>
                                                {detail.dirKeywords.join(', ')}
                                            </div>
                                        </td>
                                        <td style={{fontSize: '0.75rem'}}>
                                            {detail.matchDetails.map((match, mIdx) => {
                                                const matchName = getStoreName(match.store);
                                                return (
                                                <div key={mIdx}>
                                                    <Badge
                                                        className="me-1"
                                                        style={{
                                                            backgroundColor: STORE_COLORS[matchName] || '#455A64',
                                                            minWidth: '55px'
                                                        }}
                                                    >
                                                        {matchName}
                                                    </Badge>
                                                    <span className="text-primary">{match.bookstoreKeyword}</span>
                                                    {' ↔ '}
                                                    <span className="text-success">{match.dirKeyword}</span>
                                                    {' = '}
                                                    <strong>{match.similarity.toFixed(2)}</strong>
                                                </div>
                                                );
                                            })}
                                        </td>
                                        <td className="text-center">
                                            <Badge bg={idx === 0 ? 'success' : 'secondary'}>
                                                {detail.totalScore.toFixed(2)}
                                            </Badge>
                                        </td>
                                    </tr>
                                    );
                                })}
                            </tbody>
                        </Table>
                    </div>
                </Card.Body>
            )}
        </Card>
    );
}

SimilarityDebug.propTypes = {
    suggestedCategories: PropTypes.object,
    categoryList: PropTypes.array,
    showWhenEmpty: PropTypes.bool
};

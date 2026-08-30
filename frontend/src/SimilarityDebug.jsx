import {useState, useEffect} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Card, Badge, Table} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {getSimilarityDebugInfo} from './Actions';
import {isCacheInitialized, fetchCategoryMappings} from './categoryMappingCache';

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

export default function SimilarityDebug({suggestedCategories, categoryList}) {
    const [isOpen, setIsOpen] = useState(false);
    const [debugInfo, setDebugInfo] = useState(null);
    const [mappingsLoaded, setMappingsLoaded] = useState(false);

    // 매핑 캐시 초기화
    useEffect(() => {
        if (isCacheInitialized()) {
            setMappingsLoaded(true);
        } else {
            fetchCategoryMappings().then(() => setMappingsLoaded(true));
        }
    }, []);

    // 디버그 정보 계산
    useEffect(() => {
        if (mappingsLoaded && suggestedCategories && Object.keys(suggestedCategories).length > 0 && categoryList?.length) {
            const info = getSimilarityDebugInfo(suggestedCategories, categoryList, 10);
            setDebugInfo(info);
        } else {
            setDebugInfo(null);
        }
    }, [suggestedCategories, categoryList, mappingsLoaded]);

    if (!debugInfo) {
        return null;
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
    categoryList: PropTypes.array
};

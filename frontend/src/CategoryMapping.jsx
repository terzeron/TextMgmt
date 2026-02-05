import {useEffect, useState, useCallback} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Form, InputGroup, ListGroup, Badge, Row, Col, Spinner} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faPlus, faTrash, faSync} from '@fortawesome/free-solid-svg-icons';

import {getApiUrlPrefix} from './Common';

// 메모리 캐시 (동기 접근용)
let cachedMappings = {};
let cacheInitialized = false;

// 카테고리 매핑 데이터 로드 (동기, 캐시에서)
export const loadCategoryMappings = () => {
    return cachedMappings;
};

// 카테고리 매핑 데이터를 서버에서 가져와 캐시 갱신
export const fetchCategoryMappings = async () => {
    try {
        const response = await fetch(`${getApiUrlPrefix()}/category-mappings`);
        const json = await response.json();
        if (json.status === 'success') {
            cachedMappings = json.result || {};
            cacheInitialized = true;
            return cachedMappings;
        }
        console.error('Failed to fetch category mappings:', json.error);
        return cachedMappings;
    } catch (e) {
        console.error('Failed to fetch category mappings:', e);
        return cachedMappings;
    }
};

// 캐시 초기화 여부
export const isCacheInitialized = () => cacheInitialized;

export default function CategoryMapping({categoryList}) {
    const [mappings, setMappings] = useState({});
    const [selectedCategory, setSelectedCategory] = useState('');
    const [newKeyword, setNewKeyword] = useState('');
    const [message, setMessage] = useState('');
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState(false);

    // 서버에서 데이터 로드
    const loadFromServer = useCallback(async () => {
        setLoading(true);
        try {
            const data = await fetchCategoryMappings();
            setMappings(data);
        } finally {
            setLoading(false);
        }
    }, []);

    // 초기 로드
    useEffect(() => {
        loadFromServer();
    }, [loadFromServer]);

    // 카테고리 선택
    const handleCategorySelect = useCallback((category) => {
        setSelectedCategory(category);
        setNewKeyword('');
    }, []);

    // 키워드 추가 (서버에 즉시 저장)
    const handleAddKeyword = useCallback(async () => {
        if (!selectedCategory || !newKeyword.trim()) return;

        const keyword = newKeyword.trim();

        // 이미 존재하는지 확인
        if (mappings[selectedCategory]?.includes(keyword)) {
            setMessage('이미 등록된 키워드입니다.');
            setTimeout(() => setMessage(''), 3000);
            return;
        }

        setSaving(true);
        try {
            const response = await fetch(
                `${getApiUrlPrefix()}/category-mappings/${encodeURIComponent(selectedCategory)}/keywords`,
                {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({keyword})
                }
            );
            const json = await response.json();

            if (json.status === 'success') {
                // 로컬 상태 및 캐시 업데이트
                setMappings(prev => {
                    const updated = {...prev};
                    if (!updated[selectedCategory]) {
                        updated[selectedCategory] = [];
                    }
                    updated[selectedCategory] = [...updated[selectedCategory], keyword];
                    cachedMappings = updated;
                    return updated;
                });
                setNewKeyword('');
            } else if (json.status === 'duplicate') {
                setMessage(json.message || '이미 등록된 키워드입니다.');
                setTimeout(() => setMessage(''), 3000);
            } else {
                setMessage(json.error || '추가에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            }
        } catch (e) {
            console.error('Failed to add keyword:', e);
            setMessage('추가 중 오류가 발생했습니다.');
            setTimeout(() => setMessage(''), 3000);
        } finally {
            setSaving(false);
        }
    }, [selectedCategory, newKeyword, mappings]);

    // 키워드 삭제 (서버에 즉시 저장)
    const handleRemoveKeyword = useCallback(async (keyword) => {
        if (!selectedCategory) return;

        setSaving(true);
        try {
            const response = await fetch(
                `${getApiUrlPrefix()}/category-mappings/${encodeURIComponent(selectedCategory)}/keywords/${encodeURIComponent(keyword)}`,
                {method: 'DELETE'}
            );
            const json = await response.json();

            if (json.status === 'success') {
                // 로컬 상태 및 캐시 업데이트
                setMappings(prev => {
                    const updated = {...prev};
                    if (updated[selectedCategory]) {
                        updated[selectedCategory] = updated[selectedCategory].filter(k => k !== keyword);
                    }
                    cachedMappings = updated;
                    return updated;
                });
            } else {
                setMessage(json.error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            }
        } catch (e) {
            console.error('Failed to remove keyword:', e);
            setMessage('삭제 중 오류가 발생했습니다.');
            setTimeout(() => setMessage(''), 3000);
        } finally {
            setSaving(false);
        }
    }, [selectedCategory]);

    // Enter 키 처리
    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleAddKeyword();
        }
    }, [handleAddKeyword]);

    const currentKeywords = selectedCategory ? (mappings[selectedCategory] || []) : [];

    return (
        <Card>
            <Card.Header>
                카테고리 매핑 관리
                <Button
                    variant="outline-secondary"
                    className="btn-xs float-end"
                    onClick={loadFromServer}
                    disabled={loading}
                >
                    <FontAwesomeIcon icon={faSync} spin={loading}/> 새로고침
                </Button>
            </Card.Header>
            <Card.Body>
                {message && (
                    <div className={`alert ${message.includes('실패') || message.includes('오류') ? 'alert-danger' : 'alert-info'} py-1 mb-2`}>
                        {message}
                    </div>
                )}
                {loading ? (
                    <div className="text-center p-4">
                        <Spinner animation="border" />
                        <p className="mt-2">로딩 중...</p>
                    </div>
                ) : (
                    <Row>
                        <Col md={4}>
                            <Card>
                                <Card.Header className="py-1">디렉토리 목록</Card.Header>
                                <ListGroup variant="flush" style={{maxHeight: '400px', overflowY: 'auto'}}>
                                    {categoryList?.map(category => (
                                        <ListGroup.Item
                                            key={category}
                                            action
                                            active={selectedCategory === category}
                                            onClick={() => handleCategorySelect(category)}
                                            className="py-1 d-flex justify-content-between align-items-center"
                                        >
                                            <span style={{fontSize: '0.85rem'}}>{category}</span>
                                            {mappings[category]?.length > 0 && (
                                                <Badge bg="secondary" pill>{mappings[category].length}</Badge>
                                            )}
                                        </ListGroup.Item>
                                    ))}
                                </ListGroup>
                            </Card>
                        </Col>
                        <Col md={8}>
                            {selectedCategory ? (
                                <Card>
                                    <Card.Header className="py-1">
                                        <strong>{selectedCategory}</strong> 키워드
                                        {saving && <Spinner animation="border" size="sm" className="ms-2"/>}
                                    </Card.Header>
                                    <Card.Body>
                                        <InputGroup className="mb-2">
                                            <Form.Control
                                                type="text"
                                                placeholder="새 키워드 입력"
                                                value={newKeyword}
                                                onChange={(e) => setNewKeyword(e.target.value)}
                                                onKeyDown={handleKeyDown}
                                                disabled={saving}
                                            />
                                            <Button
                                                variant="outline-primary"
                                                onClick={handleAddKeyword}
                                                disabled={saving || !newKeyword.trim()}
                                            >
                                                <FontAwesomeIcon icon={faPlus}/> 추가
                                            </Button>
                                        </InputGroup>
                                        <div className="d-flex flex-wrap gap-1">
                                            {currentKeywords.map(keyword => (
                                                <Badge
                                                    key={keyword}
                                                    bg="info"
                                                    className="d-flex align-items-center gap-1"
                                                    style={{fontSize: '0.85rem', padding: '0.4rem 0.6rem'}}
                                                >
                                                    {keyword}
                                                    <FontAwesomeIcon
                                                        icon={faTrash}
                                                        style={{cursor: saving ? 'not-allowed' : 'pointer', marginLeft: '4px'}}
                                                        onClick={() => !saving && handleRemoveKeyword(keyword)}
                                                    />
                                                </Badge>
                                            ))}
                                            {currentKeywords.length === 0 && (
                                                <span className="text-muted">등록된 키워드가 없습니다.</span>
                                            )}
                                        </div>
                                    </Card.Body>
                                </Card>
                            ) : (
                                <div className="text-muted p-3">왼쪽에서 디렉토리를 선택하세요.</div>
                            )}
                        </Col>
                    </Row>
                )}
                <div className="mt-3 text-muted" style={{fontSize: '0.8rem'}}>
                    <strong>사용법:</strong> 각 디렉토리에 서점 카테고리와 매칭될 키워드를 등록합니다.<br/>
                    예: "4_심리학뇌과학" → "심리학", "뇌과학", "인지과학" 등<br/>
                    <em>변경 사항은 자동으로 저장됩니다.</em>
                </div>
            </Card.Body>
        </Card>
    );
}

CategoryMapping.propTypes = {
    categoryList: PropTypes.array.isRequired,
};

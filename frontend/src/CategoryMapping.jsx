import {useEffect, useState, useCallback, useRef} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Form, InputGroup, ListGroup, Badge, Row, Col, Spinner} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faPlus, faTrash, faEyeSlash, faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {jsonGetReq, jsonPostReq, jsonDeleteReq} from './Common';

// 메모리 캐시 (동기 접근용)
let cachedMappings = {};
let cacheInitialized = false;

// 카테고리 매핑 데이터 로드 (동기, 캐시에서)
export const loadCategoryMappings = () => {
    return cachedMappings;
};

// 카테고리 매핑 데이터를 서버에서 가져와 캐시 갱신
export const fetchCategoryMappings = () => {
    return new Promise((resolve) => {
        jsonGetReq('/category-mappings', null,
            (result) => {
                cachedMappings = result || {};
                cacheInitialized = true;
                resolve(cachedMappings);
            },
            (error) => {
                console.error('Failed to fetch category mappings:', error);
                // API 실패 시에도 초기화 완료로 표시 (무한 재시도 방지)
                cacheInitialized = true;
                resolve(cachedMappings);
            }
        );
    });
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
    const [hiddenCategories, setHiddenCategories] = useState(new Set());
    const [isOpen, setIsOpen] = useState(false);
    const keywordInputRef = useRef(null);

    // 비노출 카테고리 로드
    const loadHiddenCategories = useCallback(() => {
        jsonGetReq('/hidden-categories', null,
            (result) => {
                setHiddenCategories(new Set(result || []));
            },
            (error) => {
                console.error('Failed to fetch hidden categories:', error);
            }
        );
    }, []);

    // 서버에서 데이터 로드
    const loadFromServer = useCallback(async () => {
        setLoading(true);
        try {
            const data = await fetchCategoryMappings();
            setMappings(data);
            loadHiddenCategories();
        } finally {
            setLoading(false);
        }
    }, [loadHiddenCategories]);

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
    const handleAddKeyword = useCallback(() => {
        if (!selectedCategory || !newKeyword.trim()) return;

        const keyword = newKeyword.trim();

        // 이미 존재하는지 확인
        if (mappings[selectedCategory]?.includes(keyword)) {
            setMessage('이미 등록된 키워드입니다.');
            setTimeout(() => setMessage(''), 3000);
            return;
        }

        setSaving(true);
        jsonPostReq(
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords`,
            {keyword},
            () => {
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
                // 입력 필드에 포커스 유지 (setSaving 후 리렌더 완료 후 실행)
                setTimeout(() => keywordInputRef.current?.focus(), 0);
            },
            (error) => {
                setMessage(error || '이미 등록된 키워드이거나 추가에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, newKeyword, mappings]);

    // 키워드 삭제 (서버에 즉시 저장)
    const handleRemoveKeyword = useCallback((keyword) => {
        if (!selectedCategory) return;

        setSaving(true);
        jsonDeleteReq(
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords/${encodeURIComponent(keyword)}`,
            null,
            () => {
                // 로컬 상태 및 캐시 업데이트
                setMappings(prev => {
                    const updated = {...prev};
                    if (updated[selectedCategory]) {
                        updated[selectedCategory] = updated[selectedCategory].filter(k => k !== keyword);
                    }
                    cachedMappings = updated;
                    return updated;
                });
            },
            (error) => {
                setMessage(error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory]);

    // 비노출 토글
    const handleToggleHidden = useCallback((category, currentlyHidden) => {
        setSaving(true);
        jsonPostReq(
            `/hidden-categories/${category.split('/').map(encodeURIComponent).join('/')}`,
            {hidden: !currentlyHidden},
            (result) => {
                setHiddenCategories(new Set(result || []));
            },
            (error) => {
                setMessage(error || '비노출 설정 변경에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, []);

    // Enter 키 처리
    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleAddKeyword();
        }
    }, [handleAddKeyword]);

    const currentKeywords = selectedCategory ? (mappings[selectedCategory] || []) : [];

    if (!isOpen) {
        return (
            <Card>
                <Card.Header
                    onClick={() => setIsOpen(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    카테고리 관리
                </Card.Header>
            </Card>
        );
    }

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(false)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={faChevronDown} className="me-2"/>
                카테고리 관리
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
                    <Row className="g-0">
                        <Col md={4}>
                            <Card>
                                <Card.Header className="py-1">디렉토리 목록</Card.Header>
                                <ListGroup variant="flush" style={{maxHeight: '400px', overflowY: 'auto'}}>
                                    {categoryList?.map(category => {
                                        const isHidden = hiddenCategories.has(category);
                                        return (
                                            <ListGroup.Item
                                                key={category}
                                                action
                                                active={selectedCategory === category}
                                                onClick={() => handleCategorySelect(category)}
                                                className="py-1 d-flex justify-content-between align-items-center"
                                                style={isHidden ? {opacity: 0.5} : {}}
                                            >
                                                <span style={{fontSize: '0.85rem'}}>
                                                    {category}
                                                    {isHidden && (
                                                        <Badge bg="warning" text="dark" className="ms-1" style={{fontSize: '0.65rem'}}>
                                                            <FontAwesomeIcon icon={faEyeSlash} size="xs"/> 비노출
                                                        </Badge>
                                                    )}
                                                </span>
                                                {mappings[category]?.length > 0 && (
                                                    <Badge bg="secondary" pill>{mappings[category].length}</Badge>
                                                )}
                                            </ListGroup.Item>
                                        );
                                    })}
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
                                        <Form.Check
                                            type="checkbox"
                                            id={`hidden-${selectedCategory}`}
                                            label="사용자 비노출"
                                            checked={hiddenCategories.has(selectedCategory)}
                                            onChange={() => handleToggleHidden(selectedCategory, hiddenCategories.has(selectedCategory))}
                                            disabled={saving}
                                            className="mb-2"
                                        />
                                        <InputGroup className="mb-2">
                                            <Form.Control
                                                ref={keywordInputRef}
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
            </Card.Body>
        </Card>
    );
}

CategoryMapping.propTypes = {
    categoryList: PropTypes.array.isRequired,
};

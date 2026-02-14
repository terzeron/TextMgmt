import {useEffect, useState, useCallback, useRef} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Form, InputGroup, ListGroup, Badge, Row, Col, Spinner, Modal} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faPlus, faTrash, faEyeSlash, faChevronDown, faChevronRight, faEdit} from '@fortawesome/free-solid-svg-icons';

import {jsonGetReq, jsonPostReq, jsonPutReq, jsonDeleteReq} from './Common';

// 메모리 캐시 (동기 접근용) - content_type별 분리
let cachedMappings = {book: {}, comic: {}};
let cacheInitialized = {book: false, comic: false};

// 카테고리 매핑 데이터 로드 (동기, 캐시에서)
export const loadCategoryMappings = (contentType = 'book') => {
    return cachedMappings[contentType] || {};
};

// 카테고리 매핑 데이터를 서버에서 가져와 캐시 갱신
export const fetchCategoryMappings = (contentType = 'book') => {
    return new Promise((resolve) => {
        jsonGetReq(`/category-mappings?content_type=${contentType}`, null,
            (result) => {
                cachedMappings[contentType] = result || {};
                cacheInitialized[contentType] = true;
                resolve(cachedMappings[contentType]);
            },
            (error) => {
                console.error('Failed to fetch category mappings:', error);
                // API 실패 시에도 초기화 완료로 표시 (무한 재시도 방지)
                cacheInitialized[contentType] = true;
                resolve(cachedMappings[contentType]);
            }
        );
    });
};

// 캐시 초기화 여부
export const isCacheInitialized = (contentType = 'book') => cacheInitialized[contentType];

export default function CategoryMapping({categoryList, contentType = 'book', title = '카테고리 관리', onCategoryChanged}) {
    const [mappings, setMappings] = useState({});
    const [selectedCategory, setSelectedCategory] = useState('');
    const [newKeyword, setNewKeyword] = useState('');
    const [message, setMessage] = useState('');
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState(false);
    const [hiddenCategories, setHiddenCategories] = useState(new Set());
    const [isOpen, setIsOpen] = useState(false);
    const keywordInputRef = useRef(null);

    // rename/delete 모달 상태
    const [showRenameModal, setShowRenameModal] = useState(false);
    const [showDeleteModal, setShowDeleteModal] = useState(false);
    const [newCategoryName, setNewCategoryName] = useState('');

    // API prefix (comic이면 /comics 경로)
    const apiPrefix = contentType === 'comic' ? '/comics' : '';

    // 비노출 카테고리 로드
    const loadHiddenCategories = useCallback(() => {
        jsonGetReq(`/hidden-categories?content_type=${contentType}`, null,
            (result) => {
                setHiddenCategories(new Set(result || []));
            },
            (error) => {
                console.error('Failed to fetch hidden categories:', error);
            }
        );
    }, [contentType]);

    // 서버에서 데이터 로드
    const loadFromServer = useCallback(async () => {
        setLoading(true);
        try {
            const data = await fetchCategoryMappings(contentType);
            setMappings(data);
            loadHiddenCategories();
        } finally {
            setLoading(false);
        }
    }, [contentType, loadHiddenCategories]);

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
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords?content_type=${contentType}`,
            {keyword},
            () => {
                // 로컬 상태 및 캐시 업데이트
                setMappings(prev => {
                    const updated = {...prev};
                    if (!updated[selectedCategory]) {
                        updated[selectedCategory] = [];
                    }
                    updated[selectedCategory] = [...updated[selectedCategory], keyword];
                    cachedMappings[contentType] = updated;
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
    }, [selectedCategory, newKeyword, mappings, contentType]);

    // 키워드 삭제 (서버에 즉시 저장)
    const handleRemoveKeyword = useCallback((keyword) => {
        if (!selectedCategory) return;

        setSaving(true);
        jsonDeleteReq(
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords/${encodeURIComponent(keyword)}?content_type=${contentType}`,
            null,
            () => {
                // 로컬 상태 및 캐시 업데이트
                setMappings(prev => {
                    const updated = {...prev};
                    if (updated[selectedCategory]) {
                        updated[selectedCategory] = updated[selectedCategory].filter(k => k !== keyword);
                    }
                    cachedMappings[contentType] = updated;
                    return updated;
                });
            },
            (error) => {
                setMessage(error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, contentType]);

    // 비노출 토글
    const handleToggleHidden = useCallback((category, currentlyHidden) => {
        setSaving(true);
        jsonPostReq(
            `/hidden-categories/${category.split('/').map(encodeURIComponent).join('/')}?content_type=${contentType}`,
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
    }, [contentType]);

    // 카테고리 이름 변경
    const handleRenameCategory = useCallback(() => {
        if (!selectedCategory || !newCategoryName.trim()) return;
        const trimmed = newCategoryName.trim();
        if (trimmed === selectedCategory) {
            setMessage('현재 이름과 동일합니다.');
            setTimeout(() => setMessage(''), 3000);
            return;
        }

        setSaving(true);
        jsonPutReq(
            `${apiPrefix}/categories/rename`,
            {old_category: selectedCategory, new_category: trimmed},
            (result) => {
                setMessage(`카테고리 '${selectedCategory}'을(를) '${trimmed}'(으)로 변경했습니다.`);
                setTimeout(() => setMessage(''), 5000);
                setShowRenameModal(false);
                setSelectedCategory('');
                // 매핑 캐시 갱신
                loadFromServer();
                // 부모 컴포넌트에 카테고리 변경 알림
                onCategoryChanged?.();
            },
            (error) => {
                setMessage(error || '이름 변경에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, newCategoryName, apiPrefix, loadFromServer, onCategoryChanged]);

    // 카테고리 삭제
    const handleDeleteCategory = useCallback(() => {
        if (!selectedCategory) return;

        setSaving(true);
        jsonPostReq(
            `${apiPrefix}/categories/delete`,
            {category: selectedCategory},
            (result) => {
                setMessage(`카테고리 '${selectedCategory}'이(가) 삭제되었습니다. (${result.deleted_count}건)`);
                setTimeout(() => setMessage(''), 5000);
                setShowDeleteModal(false);
                setSelectedCategory('');
                // 매핑 캐시 갱신
                loadFromServer();
                // 부모 컴포넌트에 카테고리 변경 알림
                onCategoryChanged?.();
            },
            (error) => {
                setMessage(error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, apiPrefix, loadFromServer, onCategoryChanged]);

    // Enter 키 처리
    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleAddKeyword();
        }
    }, [handleAddKeyword]);

    // rename 모달 Enter 키 처리
    const handleRenameKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleRenameCategory();
        }
    }, [handleRenameCategory]);

    const isSubcategory = selectedCategory.includes('/');
    const currentKeywords = selectedCategory ? (mappings[selectedCategory] || []) : [];

    if (!isOpen) {
        return (
            <Card>
                <Card.Header
                    onClick={() => setIsOpen(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    {title}
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
                {title}
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
                                <ListGroup variant="flush">
                                    {categoryList?.map(category => {
                                        const isHidden = hiddenCategories.has(category);
                                        const depth = category.split('/').length - 1;
                                        return (
                                            <ListGroup.Item
                                                key={category}
                                                action
                                                active={selectedCategory === category}
                                                onClick={() => handleCategorySelect(category)}
                                                className="py-1 d-flex justify-content-between align-items-center"
                                                style={{
                                                    ...(isHidden ? {opacity: 0.5} : {}),
                                                    ...(depth > 0 ? {paddingLeft: `${0.75 + depth * 1.2}rem`} : {}),
                                                }}
                                            >
                                                <span style={{fontSize: '0.85rem'}}>
                                                    {depth > 0 ? category.split('/').pop() : category}
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
                                    <Card.Header className="py-1 d-flex justify-content-between align-items-center">
                                        <span>
                                            <strong>{selectedCategory}</strong>
                                            {saving && <Spinner animation="border" size="sm" className="ms-2"/>}
                                        </span>
                                        <span>
                                            <Button
                                                variant="outline-secondary"
                                                size="sm"
                                                className="me-1"
                                                disabled={saving}
                                                onClick={() => {
                                                    setNewCategoryName(selectedCategory);
                                                    setShowRenameModal(true);
                                                }}
                                                title="이름 변경"
                                            >
                                                <FontAwesomeIcon icon={faEdit}/>
                                            </Button>
                                            <Button
                                                variant="outline-danger"
                                                size="sm"
                                                disabled={saving}
                                                onClick={() => setShowDeleteModal(true)}
                                                title="카테고리 삭제"
                                            >
                                                <FontAwesomeIcon icon={faTrash}/>
                                            </Button>
                                        </span>
                                    </Card.Header>
                                    <Card.Body>
                                        <Form.Check
                                            type="checkbox"
                                            id={`hidden-${contentType}-${selectedCategory}`}
                                            label="사용자 비노출"
                                            checked={hiddenCategories.has(selectedCategory)}
                                            onChange={() => handleToggleHidden(selectedCategory, hiddenCategories.has(selectedCategory))}
                                            disabled={saving}
                                            className="mb-2"
                                        />
                                        {!isSubcategory && (
                                            <>
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
                                            </>
                                        )}
                                    </Card.Body>
                                </Card>
                            ) : (
                                <div className="text-muted p-3">왼쪽에서 디렉토리를 선택하세요.</div>
                            )}
                        </Col>
                    </Row>
                )}
            </Card.Body>

            {/* 이름 변경 모달 */}
            <Modal show={showRenameModal} onHide={() => setShowRenameModal(false)} centered>
                <Modal.Header closeButton>
                    <Modal.Title>카테고리 이름 변경</Modal.Title>
                </Modal.Header>
                <Modal.Body>
                    <Form.Group>
                        <Form.Label>현재 이름</Form.Label>
                        <Form.Control type="text" value={selectedCategory} disabled />
                    </Form.Group>
                    <Form.Group className="mt-3">
                        <Form.Label>새 이름</Form.Label>
                        <Form.Control
                            type="text"
                            value={newCategoryName}
                            onChange={(e) => setNewCategoryName(e.target.value)}
                            onKeyDown={handleRenameKeyDown}
                            autoFocus
                        />
                    </Form.Group>
                </Modal.Body>
                <Modal.Footer>
                    <Button variant="secondary" onClick={() => setShowRenameModal(false)}>취소</Button>
                    <Button
                        variant="primary"
                        onClick={handleRenameCategory}
                        disabled={saving || !newCategoryName.trim() || newCategoryName.trim() === selectedCategory}
                    >
                        {saving ? <Spinner animation="border" size="sm" /> : '변경'}
                    </Button>
                </Modal.Footer>
            </Modal>

            {/* 삭제 확인 모달 */}
            <Modal show={showDeleteModal} onHide={() => setShowDeleteModal(false)} centered>
                <Modal.Header closeButton>
                    <Modal.Title>카테고리 삭제</Modal.Title>
                </Modal.Header>
                <Modal.Body>
                    <p className="text-danger fw-bold">
                        카테고리 &apos;{selectedCategory}&apos; 및 하위 카테고리의 모든 문서가 삭제됩니다.
                    </p>
                    <p className="text-muted">이 작업은 되돌릴 수 없습니다.</p>
                </Modal.Body>
                <Modal.Footer>
                    <Button variant="secondary" onClick={() => setShowDeleteModal(false)}>취소</Button>
                    <Button
                        variant="danger"
                        onClick={handleDeleteCategory}
                        disabled={saving}
                    >
                        {saving ? <Spinner animation="border" size="sm" /> : '삭제'}
                    </Button>
                </Modal.Footer>
            </Modal>
        </Card>
    );
}

CategoryMapping.propTypes = {
    categoryList: PropTypes.array.isRequired,
    contentType: PropTypes.string,
    title: PropTypes.string,
    onCategoryChanged: PropTypes.func,
};

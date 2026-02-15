import {jsonGetReq} from './Common';

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

// 캐시 직접 갱신 (CategoryAdmin에서 키워드 추가/삭제 시 사용)
export const updateCachedMappings = (contentType, mappings) => {
    cachedMappings[contentType] = mappings;
    cacheInitialized[contentType] = true;
};

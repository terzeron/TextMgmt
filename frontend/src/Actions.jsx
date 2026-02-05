import {useEffect, useState, useMemo} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Button, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faTruckMoving, faUpload} from '@fortawesome/free-solid-svg-icons';

import {getRandomMediumColor, ROOT_DIRECTORY} from './Common';
import {loadCategoryMappings, fetchCategoryMappings, isCacheInitialized} from './CategoryMapping';


// 레벤슈타인 거리 계산
const levenshteinDistance = (str1, str2) => {
    const m = str1.length;
    const n = str2.length;
    const dp = Array(m + 1).fill(null).map(() => Array(n + 1).fill(0));

    for (let i = 0; i <= m; i++) dp[i][0] = i;
    for (let j = 0; j <= n; j++) dp[0][j] = j;

    for (let i = 1; i <= m; i++) {
        for (let j = 1; j <= n; j++) {
            if (str1[i - 1] === str2[j - 1]) {
                dp[i][j] = dp[i - 1][j - 1];
            } else {
                dp[i][j] = 1 + Math.min(dp[i - 1][j], dp[i][j - 1], dp[i - 1][j - 1]);
            }
        }
    }
    return dp[m][n];
};

// 레벤슈타인 유사도 (0~1, 1이 완전 일치)
const levenshteinSimilarity = (str1, str2) => {
    if (!str1 || !str2) return 0;
    const s1 = str1.toLowerCase();
    const s2 = str2.toLowerCase();
    const distance = levenshteinDistance(s1, s2);
    const maxLen = Math.max(s1.length, s2.length);
    return maxLen === 0 ? 1 : 1 - distance / maxLen;
};

// 유사도 계산 (레벤슈타인 + 포함 보너스)
// 포함 관계가 있으면 높은 점수, 그렇지 않으면 레벤슈타인 유사도
const calculateSimilarity = (str1, str2) => {
    if (!str1 || !str2) return 0;
    const s1 = str1.toLowerCase();
    const s2 = str2.toLowerCase();

    // 완전 일치
    if (s1 === s2) return 1.0;

    // 포함 관계 체크 - 짧은 문자열이 2글자 이상이면 포함 보너스
    const shorter = s1.length <= s2.length ? s1 : s2;
    const longer = s1.length > s2.length ? s1 : s2;

    if (longer.includes(shorter) && shorter.length >= 2) {
        // 비율에 따라 점수 차등 (0.6 ~ 0.9)
        const ratio = shorter.length / longer.length;
        return 0.6 + (ratio * 0.3); // 비율 100%면 0.9, 33%면 0.7
    }

    return levenshteinSimilarity(s1, s2);
};

// 특수기호로 문자열을 분리하여 키워드 추출
// 슬래시(/), 괄호(()), 공백 등을 구분자로 사용
const splitBySpecialChars = (str) => {
    if (!str) return [];
    // 슬래시, 괄호, 공백 등으로 분리 후 빈 문자열 제거
    return str.split(/[\/\(\)\s]+/).map(s => s.trim()).filter(s => s.length >= 2);
};

// 단일 서점 카테고리와 디렉토리의 유사도 점수 계산
// - 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체
// - 서점 카테고리(이미 마지막 레벨)를 특수기호로 분리하여 사용
// - { category: score } 형태의 객체 반환
const calculateCategoryScores = (bookstoreCategory, categoryList) => {
    if (!bookstoreCategory || !categoryList?.length) return {};

    // 서점 카테고리를 특수기호로 분리하여 키워드 추출
    const deepKeywords = splitBySpecialChars(bookstoreCategory);

    if (deepKeywords.length === 0) return {};

    const mappings = loadCategoryMappings();
    const scores = {};

    for (const category of categoryList) {
        // 디렉토리명에서 숫자 prefix 제거 (예: "4_심리학뇌과학" -> "심리학뇌과학")
        const categoryName = category.includes('_')
            ? category.split('_').slice(1).join('_')
            : category;

        // 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체
        const dirKeywords = [...(mappings[category] || []), categoryName];

        // 서점 카테고리와 디렉토리 키워드 간 최대 유사도 계산
        let maxSimilarity = 0;
        for (const deepKeyword of deepKeywords) {
            for (const dirKeyword of dirKeywords) {
                const similarity = calculateSimilarity(deepKeyword, dirKeyword);
                if (similarity > maxSimilarity) {
                    maxSimilarity = similarity;
                }
            }
        }

        // 모든 카테고리의 유사도 저장 (0보다 크면)
        if (maxSimilarity > 0) {
            scores[category] = maxSimilarity;
        }
    }

    return scores;
};

// 여러 서점 카테고리의 유사도를 합산하여 상위 N개 반환
// - suggestedCategories: { yes24: 'category1', aladin: 'category2', ... }
const findTopSimilarCategories = (suggestedCategories, categoryList, topN = 3) => {
    if (!suggestedCategories || !categoryList?.length) return [];

    const combinedScores = {};

    // 각 서점의 카테고리에 대해 유사도 점수 계산 후 합산
    for (const [store, bookstoreCategory] of Object.entries(suggestedCategories)) {
        if (!bookstoreCategory) continue;

        const storeScores = calculateCategoryScores(bookstoreCategory, categoryList);
        for (const [category, score] of Object.entries(storeScores)) {
            combinedScores[category] = (combinedScores[category] || 0) + score;
        }
    }

    // 점수 내림차순 정렬 후 상위 N개 반환
    return Object.entries(combinedScores)
        .sort((a, b) => b[1] - a[1])
        .slice(0, topN)
        .map(item => item[0]);
};

// 디버깅용: 상위 N개 카테고리의 유사도 계산 과정 반환
export const getSimilarityDebugInfo = (suggestedCategories, categoryList, topN = 10) => {
    if (!suggestedCategories || !categoryList?.length) return null;

    const mappings = loadCategoryMappings();
    const debugInfo = {
        bookstoreKeywords: {}, // 서점별 추출된 키워드
        categoryDetails: [],   // 각 카테고리별 상세 계산 과정
    };

    // 1. 서점별 추출된 키워드 수집 (이미 마지막 레벨만 전달됨)
    for (const [store, bookstoreCategory] of Object.entries(suggestedCategories)) {
        if (!bookstoreCategory) continue;
        const keywords = splitBySpecialChars(bookstoreCategory);
        debugInfo.bookstoreKeywords[store] = {
            original: bookstoreCategory,
            keywords
        };
    }

    // 2. 각 카테고리별 상세 점수 계산
    const allCategoryScores = {};

    for (const category of categoryList) {
        const categoryName = category.includes('_')
            ? category.split('_').slice(1).join('_')
            : category;
        const dirKeywords = [...(mappings[category] || []), categoryName];

        const matchDetails = []; // 어떤 키워드 쌍이 매칭되었는지
        let totalScore = 0;

        for (const [store, storeInfo] of Object.entries(debugInfo.bookstoreKeywords)) {
            let maxSimilarity = 0;
            let bestMatch = null;

            for (const deepKeyword of storeInfo.keywords) {
                for (const dirKeyword of dirKeywords) {
                    const similarity = calculateSimilarity(deepKeyword, dirKeyword);
                    if (similarity > maxSimilarity) {
                        maxSimilarity = similarity;
                        bestMatch = {
                            bookstoreKeyword: deepKeyword,
                            dirKeyword,
                            similarity,
                            isContained: dirKeyword.toLowerCase().includes(deepKeyword.toLowerCase()) ||
                                         deepKeyword.toLowerCase().includes(dirKeyword.toLowerCase())
                        };
                    }
                }
            }

            if (maxSimilarity > 0) {
                matchDetails.push({ store, ...bestMatch });
                totalScore += maxSimilarity;
            }
        }

        if (totalScore > 0) {
            allCategoryScores[category] = {
                categoryName,
                dirKeywords,
                totalScore,
                matchDetails
            };
        }
    }

    // 3. 상위 N개만 추출
    debugInfo.categoryDetails = Object.entries(allCategoryScores)
        .sort((a, b) => b[1].totalScore - a[1].totalScore)
        .slice(0, topN)
        .map(([category, details]) => ({ category, ...details }));

    return debugInfo;
};

export default function Actions(props) {
    const [renderingInfoList, setRenderingInfoList] = useState([]);
    const [mappingsLoaded, setMappingsLoaded] = useState(false);

    // 매핑 캐시 초기화
    useEffect(() => {
        if (isCacheInitialized()) {
            // 이미 다른 곳에서 캐시가 초기화됨
            setMappingsLoaded(true);
        } else {
            fetchCategoryMappings().then(() => setMappingsLoaded(true));
        }
    }, []);

    // 여러 서점 카테고리와 유사한 상위 5개 디렉토리 찾기 (useMemo로 동기 계산)
    const highlightedCategories = useMemo(() => {
        if (props.suggestedCategories && Object.keys(props.suggestedCategories).length > 0 && props.otherCategoryList?.length && mappingsLoaded) {
            return findTopSimilarCategories(props.suggestedCategories, props.otherCategoryList, 5);
        }
        return [];
    }, [props.suggestedCategories, props.otherCategoryList, mappingsLoaded]);

    // 유사도 1위 카테고리가 변경되면 자동으로 선택
    useEffect(() => {
        if (highlightedCategories.length > 0 && props.selectDirectoryButtonClicked) {
            props.selectDirectoryButtonClicked(null, highlightedCategories[0]);
        }
    }, [highlightedCategories, props.selectDirectoryButtonClicked]);

    useEffect(() => {
        const infoList = props.otherCategoryList?.map(category => {
            const hasSubCategory = category.includes('_');
            if (hasSubCategory) {
                const prefix = category.split('_')[0];
                const subCategory = category.split('_')[1];
                return {key: category, label: subCategory, style: {backgroundColor: getRandomMediumColor(prefix), color: 'white'}};
            }
            return {key: category, label: category, style: {}, class: 'btn-light'};
        });
        setRenderingInfoList(infoList);
    }, [props.otherCategoryList]);

    return (
        <>
            <Row className="button_group">
                <Button variant="outline-success" className="btn-xs" onClick={props.toNextEntryClicked}>다음 책으로</Button>
                {
                    !props.selectedEntryId.startsWith(ROOT_DIRECTORY) > 0 &&
                    <Button variant="outline-warning" className="btn-xs" onClick={props.moveToUpperButtonClicked} disabled={!props.newFileName}>
                        상위로
                        <FontAwesomeIcon icon={faUpload}/>
                    </Button>
                }
                {
                    renderingInfoList.map(info => {
                        const highlightRank = Array.isArray(highlightedCategories) ? highlightedCategories.indexOf(info['key']) : -1;
                        const isTop1 = highlightRank === 0;
                        const isTop2to5 = highlightRank >= 1 && highlightRank <= 4;
                        const highlightClass = isTop1 ? 'highlight' : isTop2to5 ? 'highlight-secondary' : '';
                        return (
                            <Button
                                variant="outline-secondary"
                                key={info['key']}
                                className={`btn-xs ${info['class'] || ''} ${highlightClass}`}
                                style={highlightClass ? {} : info['style']}
                                onClick={(e) => {
                                    props.selectDirectoryButtonClicked(e, info['key']);
                                }}>
                                {info['label']}
                            </Button>
                        );
                    })
                }
            </Row>

            <Row>
                <InputGroup className="ms-0 me-0">
                    <Form.Control value={props.selectedCategory} readOnly/>
                    <Button variant="outline-warning" className="btn-xs" onClick={props.moveToDirectoryButtonClicked
                    } disabled={!props.selectedEntryId && !props.selectedCategory}>
                        로 옮기기
                        <FontAwesomeIcon icon={faTruckMoving}/>
                    </Button>
                </InputGroup>
            </Row>
        </>
    )
        ;
}

Actions.propTypes = {
    selectedEntryId: PropTypes.string.isRequired,
    selectedCategory: PropTypes.string.isRequired,
    otherCategoryList: PropTypes.array.isRequired,
    moveToUpperButtonClicked: PropTypes.func,
    moveToDirectoryButtonClicked: PropTypes.func,
    selectDirectoryButtonClicked: PropTypes.func,
    newFileName: PropTypes.string.isRequired,
    toNextEntryClicked: PropTypes.func.isRequired,
    suggestedCategories: PropTypes.object,
};

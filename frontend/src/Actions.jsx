import {useEffect, useState, useMemo} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Button, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faTruckMoving, faUpload} from '@fortawesome/free-solid-svg-icons';

import {getRandomMediumColor, ROOT_DIRECTORY} from './Common';
import {loadCategoryMappings, fetchCategoryMappings, isCacheInitialized} from './CategoryMapping';


// 문자열에서 n-gram 집합 생성
export const generateNgrams = (str, n = 2) => {
    const ngrams = new Set();
    for (let i = 0; i <= str.length - n; i++) {
        ngrams.add(str.substring(i, i + n));
    }
    return ngrams;
};

// N-gram 기반 Jaccard 유사도 (0~1)
export const ngramSimilarity = (str1, str2, n = 2) => {
    if (str1.length < n || str2.length < n) {
        return str1 === str2 ? 1.0 : 0;
    }
    const ngrams1 = generateNgrams(str1, n);
    const ngrams2 = generateNgrams(str2, n);
    let intersectionSize = 0;
    for (const ng of ngrams1) {
        if (ngrams2.has(ng)) intersectionSize++;
    }
    const unionSize = ngrams1.size + ngrams2.size - intersectionSize;
    return unionSize === 0 ? 0 : intersectionSize / unionSize;
};

// 한글 문자 포함 여부 확인
const containsKorean = (str) => /[\uAC00-\uD7AF]/.test(str);

// 유사도 계산 (완전일치 > 포함관계 > N-gram 유사도)
export const calculateSimilarity = (str1, str2) => {
    if (!str1 || !str2) return 0;
    const s1 = str1.toLowerCase();
    const s2 = str2.toLowerCase();

    // 1) 완전 일치 → 1.0
    if (s1 === s2) return 1.0;

    // 2) 포함 관계 → 0.7~0.9
    //    단, 한글 키워드가 우연히 포함되는 경우를 배제:
    //    - 한글 1글자가 3글자 이상 단어에 포함 → N-gram (예: "시" ⊂ "러시아소설")
    //    - 한글 2글자 이상이 절반 이하로 포함 → N-gram (예: "소설" ⊂ "세계각국소설")
    //    - 한글 1글자가 2글자 단어에 포함 → 포함 관계 유지 (예: "소" ⊂ "소설")
    const shorter = s1.length <= s2.length ? s1 : s2;
    const longer = s1.length > s2.length ? s1 : s2;
    // 한글은 1글자도 의미있으므로 허용, 그 외는 2글자 이상
    const minLength = containsKorean(shorter) ? 1 : 2;
    if (longer.includes(shorter) && shorter.length >= minLength) {
        const ratio = shorter.length / longer.length;
        if (containsKorean(shorter) && (
            (shorter.length === 1 && longer.length > 2) ||
            (shorter.length >= 2 && ratio <= 0.5)
        )) {
            // 한글 우연적 포함 → N-gram으로 평가
        } else {
            return 0.7 + (ratio * 0.2);
        }
    }

    // 3) N-gram 유사도 기반 → 0.3~0.6
    const ngramScore = ngramSimilarity(s1, s2, 2);
    const minNgramScore = containsKorean(s1) || containsKorean(s2) ? 0.15 : 0.2;
    if (ngramScore >= minNgramScore) {
        return 0.3 + (ngramScore * 0.3);
    }

    return 0;
};

// 특수기호로 문자열을 분리하여 키워드 추출
// 슬래시(/), 괄호(()), 공백 등을 구분자로 사용
const splitBySpecialChars = (str) => {
    if (!str) return [];
    // 슬래시, 괄호, 공백 등으로 분리 후 빈 문자열 제거
    return str.split(/[\/\(\)\s]+/).map(s => s.trim()).filter(Boolean);
};

// 다른 키워드에 포함되는 짧은 키워드를 제거하여 구체적인 키워드만 사용
// 예: ["소설", "세계각국소설"] → ["세계각국소설"] ("소설"은 "세계각국소설"에 포함)
export const filterSubstringKeywords = (keywords) => {
    if (keywords.length <= 1) return keywords;
    return keywords.filter(kw => {
        const kwLower = kw.toLowerCase();
        return !keywords.some(other => {
            if (other === kw) return false;
            return other.toLowerCase().includes(kwLower);
        });
    });
};

// 유사도 비교에 노이즈를 주는 부분문자열 제거
// 예: "경영일반" → "경영", "중문일반" → "중문"
const NOISE_PATTERNS = [/일반/g];

export const stripNoiseWords = (keywords) => {
    return keywords
        .map(kw => NOISE_PATTERNS.reduce((s, pat) => s.replace(pat, ''), kw).trim())
        .filter(Boolean);
};

// 단일 서점 카테고리와 디렉토리의 유사도 점수 계산
// - 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체
// - 서점 카테고리(이미 마지막 레벨)를 특수기호로 분리하여 사용
// - 다른 키워드에 포함되는 범용 키워드는 제거 (예: "소설"이 "세계각국소설"에 포함)
// - 각 키워드별 최선 매칭의 평균으로 점수 산출 (매칭 건수에 의한 편향 방지)
// - { category: score } 형태의 객체 반환
const calculateCategoryScores = (bookstoreCategory, categoryList) => {
    if (!bookstoreCategory || !categoryList?.length) return {};

    // 서점 카테고리를 특수기호로 분리 후 부분문자열 키워드 제거 + 노이즈 제거
    const rawKeywords = splitBySpecialChars(bookstoreCategory);
    const deepKeywords = stripNoiseWords(filterSubstringKeywords(rawKeywords));

    if (deepKeywords.length === 0) return {};

    const mappings = loadCategoryMappings();
    const scores = {};

    for (const category of categoryList) {
        // 디렉토리명에서 숫자 prefix 제거 (예: "4_심리학뇌과학" -> "심리학뇌과학")
        const categoryName = category.includes('_')
            ? category.split('_').slice(1).join('_')
            : category;

        // 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체 (노이즈 제거)
        const dirKeywords = stripNoiseWords([...(mappings[category] || []), categoryName]);

        // 각 서점 키워드별 최대 유사도를 구한 뒤 평균
        let totalScore = 0;
        for (const deepKeyword of deepKeywords) {
            let bestForKeyword = 0;
            for (const dirKeyword of dirKeywords) {
                const similarity = calculateSimilarity(deepKeyword, dirKeyword);
                if (similarity > bestForKeyword) {
                    bestForKeyword = similarity;
                }
            }
            totalScore += bestForKeyword;
        }

        const avgScore = totalScore / deepKeywords.length;
        if (avgScore > 0) {
            scores[category] = avgScore;
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

    // 1. 서점별 추출된 키워드 수집 (부분문자열 필터링 + 노이즈 제거)
    for (const [store, bookstoreCategory] of Object.entries(suggestedCategories)) {
        if (!bookstoreCategory) continue;
        const rawKeywords = splitBySpecialChars(bookstoreCategory);
        const keywords = stripNoiseWords(filterSubstringKeywords(rawKeywords));
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
        const dirKeywords = stripNoiseWords([...(mappings[category] || []), categoryName]);

        const matchDetails = []; // 어떤 키워드 쌍이 매칭되었는지
        let totalScore = 0;

        for (const [store, storeInfo] of Object.entries(debugInfo.bookstoreKeywords)) {
            // 각 서점 키워드별 최선 매칭을 개별적으로 기록, 평균 산출
            let storeTotal = 0;
            for (const deepKeyword of storeInfo.keywords) {
                let bestSimilarity = 0;
                let bestMatch = null;

                for (const dirKeyword of dirKeywords) {
                    const similarity = calculateSimilarity(deepKeyword, dirKeyword);
                    if (similarity > bestSimilarity) {
                        bestSimilarity = similarity;
                        const dkLower = deepKeyword.toLowerCase();
                        const drLower = dirKeyword.toLowerCase();
                        const isExact = dkLower === drLower;
                        const isContained = drLower.includes(dkLower) || dkLower.includes(drLower);
                        bestMatch = {
                            bookstoreKeyword: deepKeyword,
                            dirKeyword,
                            similarity,
                            isContained,
                            matchType: isExact ? 'exact' : isContained ? 'contains' : 'ngram'
                        };
                    }
                }

                if (bestSimilarity > 0) {
                    matchDetails.push({ store, ...bestMatch });
                }
                storeTotal += bestSimilarity;
            }
            totalScore += storeTotal / storeInfo.keywords.length;
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
    const [manuallyClicked, setManuallyClicked] = useState(false);
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
            const filteredCategoryList = props.otherCategoryList.filter(cat => cat !== '_root');
            return findTopSimilarCategories(props.suggestedCategories, filteredCategoryList, 5);
        }
        return [];
    }, [props.suggestedCategories, props.otherCategoryList, mappingsLoaded]);

    // 유사도 1위 카테고리가 변경되면 자동으로 선택 (수동 클릭 상태 초기화)
    useEffect(() => {
        if (highlightedCategories.length > 0 && props.selectDirectoryButtonClicked) {
            setManuallyClicked(false);
            props.selectDirectoryButtonClicked(null, highlightedCategories[0]);
        }
    }, [highlightedCategories, props.selectDirectoryButtonClicked]);

    useEffect(() => {
        const infoList = props.otherCategoryList
            ?.filter(category => category !== '_root')  // _root 카테고리 제외
            ?.map(category => {
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
                <Button variant="outline-success" className="btn-xs" onClick={props.toNextEntryClicked} disabled={props.isProcessing}>다음 책으로</Button>
                {
                    !props.selectedEntryId.startsWith(ROOT_DIRECTORY) > 0 &&
                    <Button variant="outline-warning" className="btn-xs" onClick={props.moveToUpperButtonClicked} disabled={!props.newFileName || props.isProcessing}>
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
                        const isSelected = info['key'] === props.selectedCategory;
                        const keepHighlight = isSelected && !manuallyClicked && highlightClass;
                        const buttonStyle = isSelected
                            ? (keepHighlight ? {} : {backgroundColor: '#fff', color: '#333'})
                            : highlightClass ? {} : info['style'];
                        return (
                            <Button
                                variant="outline-secondary"
                                key={info['key']}
                                className={`btn-sm ${info['class'] || ''} ${isSelected && !keepHighlight ? '' : highlightClass}`}
                                style={buttonStyle}
                                onClick={(e) => {
                                    setManuallyClicked(true);
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
                    } disabled={(!props.selectedEntryId && !props.selectedCategory) || props.isProcessing}>
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
    isProcessing: PropTypes.bool,
};

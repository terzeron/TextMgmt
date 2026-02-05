import {useEffect, useState} from "react";
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
// 포함 관계가 있으면 최소 0.8, 그렇지 않으면 레벤슈타인 유사도
const calculateSimilarity = (str1, str2) => {
    if (!str1 || !str2) return 0;
    const s1 = str1.toLowerCase();
    const s2 = str2.toLowerCase();

    // 포함 관계 체크 (짧은 문자열이 긴 문자열에 포함되면 0.8)
    if (s1.includes(s2) || s2.includes(s1)) {
        return 0.8;
    }

    return levenshteinSimilarity(s1, s2);
};

// 서점 카테고리와 디렉토리의 유사도 계산
// - 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체
// - 서점 카테고리의 3, 4레벨 키워드와 비교
// - 상위 N개 카테고리 반환
const findTopSimilarCategories = (bookstoreCategory, categoryList, topN = 3) => {
    if (!bookstoreCategory || !categoryList?.length) return [];

    // 서점 카테고리를 '>'로 분리 (예: "국내도서>인문학>심리학>심리학 일반")
    const categoryParts = bookstoreCategory.split('>').map(s => s.trim());
    // 3, 4 레벨 키워드만 사용 (더 구체적인 분류)
    const deepKeywords = categoryParts.slice(2); // index 2, 3, ... (3레벨 이상)

    if (deepKeywords.length === 0) return [];

    const mappings = loadCategoryMappings();
    const scores = [];

    for (const category of categoryList) {
        // 디렉토리명에서 숫자 prefix 제거 (예: "4_심리학뇌과학" -> "심리학뇌과학")
        const categoryName = category.includes('_')
            ? category.split('_').slice(1).join('_')
            : category;

        // 디렉토리별 키워드 = 매핑 테이블 키워드 + 디렉토리명 자체
        const dirKeywords = [...(mappings[category] || []), categoryName];

        // 서점 3,4레벨 키워드와 디렉토리 키워드 간 최대 유사도 계산
        let maxSimilarity = 0;
        for (const deepKeyword of deepKeywords) {
            for (const dirKeyword of dirKeywords) {
                const similarity = calculateSimilarity(deepKeyword, dirKeyword);
                if (similarity > maxSimilarity) {
                    maxSimilarity = similarity;
                }
            }
        }

        if (maxSimilarity >= 0.4) { // 최소 40% 유사도 이상
            scores.push({ category, score: maxSimilarity });
        }
    }

    // 점수 내림차순 정렬 후 상위 N개 반환
    return scores
        .sort((a, b) => b.score - a.score)
        .slice(0, topN)
        .map(item => item.category);
};

export default function Actions(props) {
    const [renderingInfoList, setRenderingInfoList] = useState([]);
    const [highlightedCategories, setHighlightedCategories] = useState([]);
    const [mappingsLoaded, setMappingsLoaded] = useState(isCacheInitialized());

    // 매핑 캐시 초기화
    useEffect(() => {
        if (!isCacheInitialized()) {
            fetchCategoryMappings().then(() => setMappingsLoaded(true));
        }
    }, []);

    // Yes24 카테고리와 유사한 상위 3개 디렉토리 찾기
    useEffect(() => {
        if (props.suggestedCategory && props.otherCategoryList?.length && mappingsLoaded) {
            const topCategories = findTopSimilarCategories(props.suggestedCategory, props.otherCategoryList, 3);
            setHighlightedCategories(topCategories);
        } else {
            setHighlightedCategories([]);
        }
    }, [props.suggestedCategory, props.otherCategoryList, mappingsLoaded]);

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
                        const isHighlighted = Array.isArray(highlightedCategories) && highlightedCategories.includes(info['key']);
                        return (
                            <Button
                                variant="outline-secondary"
                                key={info['key']}
                                className={`btn-xs ${info['class'] || ''} ${isHighlighted ? 'highlight' : ''}`}
                                style={isHighlighted ? {} : info['style']}
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
    suggestedCategory: PropTypes.string,
};

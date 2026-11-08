import { useEffect, useState, useRef } from "react";
import PropTypes from "prop-types";

import "./Edit.css";
import "bootstrap/dist/css/bootstrap.min.css";

import { Button, Form, InputGroup, Row } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faTruckMoving,
  faUpload,
  faSpinner,
} from "@fortawesome/free-solid-svg-icons";

import { getRandomLightColor, jsonPostReq, ROOT_DIRECTORY } from "./Common";

export default function Actions(props) {
  const [renderingInfoList, setRenderingInfoList] = useState([]);
  // 사용자가 직접 고른 카테고리인지. 서점 검색 결과가 늦게 도착해도 사용자의 선택이 우선이다.
  // effect가 최신 값을 읽어야 하므로 화면 표시용 state와 함께 ref로도 둔다.
  const [manuallyClicked, setManuallyClicked] = useState(false);
  const manuallyClickedRef = useRef(false);
  const [highlightedCategories, setHighlightedCategories] = useState([]);
  const suggestionRequestRef = useRef(0);

  // 서점 카테고리와 유사한 상위 5개 디렉토리는 backend(store_category_matcher)가 계산한다.
  // 새 요청을 보낼 때 이전 강조를 비우고, 늦게 도착한 이전 요청의 응답은 버린다.
  useEffect(() => {
    const requestId = ++suggestionRequestRef.current;
    const suggested = props.suggestedCategories;
    const categories = (props.otherCategoryList || []).filter(
      (cat) => cat !== "_root",
    );
    setHighlightedCategories([]);
    if (!suggested || Object.keys(suggested).length === 0 || !categories.length) {
      return;
    }
    jsonPostReq(
      "/bookstore/category-suggestions?content_type=book",
      { suggested, categories, top_n: 5 },
      (result) => {
        if (requestId === suggestionRequestRef.current) {
          setHighlightedCategories(result?.top || []);
        }
      },
      () => {},
    );
  }, [props.suggestedCategories, props.otherCategoryList]);

  // 다른 책으로 바뀌면 이전 책에서 고른 카테고리는 더 이상 유효하지 않으므로 자동 선택을 다시 허용한다.
  // 아래 자동 선택 effect보다 먼저 선언해야 같은 렌더에서 먼저 실행된다.
  useEffect(() => {
    manuallyClickedRef.current = false;
    setManuallyClicked(false);
  }, [props.selectedEntryId]);

  // 유사도 1위 카테고리가 변경되면 자동으로 선택한다.
  // 사용자가 직접 고른 뒤에는 건너뛴다. 서점 검색은 수 초가 걸려 클릭 뒤에 끝나기도 하고,
  // 카테고리 목록 재적재로 결과 배열이 새로 만들어져도 이 effect가 다시 실행되기 때문이다.
  useEffect(() => {
    if (manuallyClickedRef.current) return;
    if (
      highlightedCategories.length > 0 &&
      props.selectDirectoryButtonClicked
    ) {
      props.selectDirectoryButtonClicked(null, highlightedCategories[0]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- props 전체를 deps에 넣으면 무관한 prop 변경 시에도 재실행되므로 사용 필드만 명시 (의도된 누락)
  }, [highlightedCategories, props.selectDirectoryButtonClicked]);

  useEffect(() => {
    const infoList = props.otherCategoryList
      ?.filter((category) => category !== "_root") // _root 카테고리 제외
      ?.map((category) => {
        const hasSubCategory = category.includes("_");
        if (hasSubCategory) {
          const prefix = category.split("_")[0];
          const subCategory = category.split("_")[1];
          return {
            key: category,
            label: subCategory,
            style: {
              backgroundColor: getRandomLightColor(prefix),
              color: "#333",
            },
          };
        }
        return {
          key: category,
          label: category,
          style: {},
          class: "btn-light",
        };
      });
    setRenderingInfoList(infoList || []);
  }, [props.otherCategoryList]);

  return (
    <>
      <Row className="button_group actions-area">
        {props.directoryMode && props.showDirectoryNavigation && <Button
          variant="outline-warning"
          className="btn-xs"
          onClick={props.onPreviousDirectory}
          disabled={props.isProcessing || props.previousDirectoryDisabled}
        >
          이전 디렉토리로
        </Button>}
        {props.directoryMode && props.showDirectoryNavigation && <Button
          variant="outline-warning"
          className="btn-xs"
          onClick={props.onNextDirectory}
          disabled={props.isProcessing || props.nextDirectoryDisabled}
        >
          다음 디렉토리로
        </Button>}
        {!props.directoryMode && <Button
          variant="outline-warning"
          className="btn-xs"
          onClick={props.toPrevEntryClicked}
          disabled={props.isProcessing}
        >
          이전 책으로
          {props.isProcessing && <FontAwesomeIcon icon={faSpinner} spin />}
        </Button>}
        {!props.directoryMode && <Button
          variant="outline-warning"
          className="btn-xs"
          onClick={props.toNextEntryClicked}
          disabled={props.isProcessing}
        >
          다음 책으로
          {props.isProcessing && <FontAwesomeIcon icon={faSpinner} spin />}
        </Button>}
        {!props.directoryMode && !props.selectedEntryId?.startsWith(ROOT_DIRECTORY) && (
          <Button
            variant="outline-warning"
            className="btn-xs"
            onClick={props.moveToUpperButtonClicked}
            disabled={!props.newFileName || props.isProcessing}
          >
            상위로
            <FontAwesomeIcon
              icon={props.isProcessing ? faSpinner : faUpload}
              spin={props.isProcessing}
            />
          </Button>
        )}
        {renderingInfoList.map((info) => {
          const highlightRank = highlightedCategories.indexOf(info["key"]);
          const isTop1 = highlightRank === 0;
          const isTop2to5 = highlightRank >= 1 && highlightRank <= 4;
          const highlightClass = isTop1
            ? "highlight"
            : isTop2to5
              ? "highlight-secondary"
              : "";
          const isSelected = info["key"] === props.selectedCategory;
          const keepHighlight =
            isSelected && !manuallyClicked && highlightClass;
          const buttonStyle = isSelected
            ? keepHighlight
              ? {}
              : { backgroundColor: "#fff", color: "#333" }
            : highlightClass
              ? {}
              : info["style"];
          return (
            <Button
              variant="outline-secondary"
              key={info["key"]}
              className={`btn-sm ${info["class"] || ""} ${isSelected && !keepHighlight ? "" : highlightClass}`}
              style={buttonStyle}
              onClick={(e) => {
                manuallyClickedRef.current = true;
                setManuallyClicked(true);
                props.selectDirectoryButtonClicked(e, info["key"]);
              }}
            >
              {info["label"]}
            </Button>
          );
        })}
      </Row>

      <Row className="actions-area">
        <InputGroup className="ms-0 me-0">
          <Form.Control value={props.selectedCategory} readOnly placeholder="이동할 카테고리를 선택하세요" />
          <Button
            variant="outline-warning"
            className="btn-xs"
            onClick={props.moveToDirectoryButtonClicked}
            disabled={
              (props.directoryMode
                ? !props.selectedCategory
                : !props.selectedEntryId && !props.selectedCategory) ||
              props.isProcessing
            }
          >
            {props.directoryMode ? "디렉토리 이동" : "로 옮기기"}
            <FontAwesomeIcon
              icon={props.isProcessing ? faSpinner : faTruckMoving}
              spin={props.isProcessing}
            />
          </Button>
        </InputGroup>
      </Row>
    </>
  );
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
  toPrevEntryClicked: PropTypes.func,
  suggestedCategories: PropTypes.object,
  isProcessing: PropTypes.bool,
  directoryMode: PropTypes.bool,
  showDirectoryNavigation: PropTypes.bool,
  previousDirectoryDisabled: PropTypes.bool,
  nextDirectoryDisabled: PropTypes.bool,
  onPreviousDirectory: PropTypes.func,
  onNextDirectory: PropTypes.func,
};

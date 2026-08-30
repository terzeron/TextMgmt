import {useEffect, useState, useMemo, Suspense} from "react";
import PropTypes from "prop-types";
import {textGetReq} from "./Common";
import {formatText} from './textFormatter';
import './ViewTXT.css';

export default function ViewTXT({bookId, lineCount, apiPrefix = ''}) {
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);
    const [fileContent, setFileContent] = useState([]);
    const [merged, setMerged] = useState(false);
    const [minBlank, setMinBlank] = useState(1);

    useEffect(() => {
        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const downloadUrl = apiPrefix + "/download/" + bookId;
        setIsLoading(true);
        setErrorMessage(null);

        textGetReq(
            downloadUrl,
            null,
            (result) => {
                const text = typeof result === "string" ? result : String(result || "");
                const lines = text.split("\n");
                const lineList = lineCount > 0 ? lines.slice(0, lineCount) : lines;
                setFileContent(lineList);
                setIsLoading(false);
            },
            (error) => {
                setErrorMessage(`❌ 파일을 불러올 수 없습니다: ${error}`);
                setIsLoading(false);
            }
        );

        return () => {
            setFileContent([]);
        };
    }, [bookId, lineCount, apiPrefix]);

    const blocks = useMemo(
        () => formatText(fileContent, { minBlankLines: minBlank }),
        [fileContent, minBlank]
    );

    function renderBlock(block, index) {
        switch (block.type) {
            case 'separator':
                return <hr key={index} className="txt-separator" />;
            case 'header':
                return <p key={index} className="txt-header">{block.text}</p>;
            case 'dialogue':
                return <p key={index} className="txt-dialogue">{block.text}</p>;
            default:
                return <p key={index} className="txt-narrative">{block.text}</p>;
        }
    }

    return (
        <div className="txt-container">
            {isLoading && (
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">로딩 중...</span>
                </div>
            )}
            {errorMessage && (
                <div className="error-message">
                    {errorMessage}
                </div>
            )}
            {!isLoading && fileContent.length > 0 && (
                <div className="txt-toolbar">
                    <button
                        className={`txt-merge-btn ${merged ? 'active' : ''}`}
                        onClick={() => setMerged(prev => !prev)}
                    >
                        <span className="toggle-track"><span className="toggle-knob" /></span>
                        줄 합치기
                    </button>
                    {merged && (
                        <>
                            <div className="txt-blank-control">
                                <span className="txt-blank-label">빈 줄 기준</span>
                                <button className="txt-blank-btn" onClick={() => setMinBlank(prev => Math.max(1, prev - 1))} disabled={minBlank <= 1}>−</button>
                                <span className="txt-blank-value">{minBlank}</span>
                                <button className="txt-blank-btn" onClick={() => setMinBlank(prev => prev + 1)}>+</button>
                            </div>
                        </>
                    )}
                </div>
            )}
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                {!isLoading && !merged &&
                    fileContent.map((line, index) => (
                        <div key={index}>{line}</div>
                    ))
                }
                {!isLoading && merged &&
                    blocks.map((block, index) => renderBlock(block, index))
                }
            </Suspense>
        </div>
    );
}

ViewTXT.propTypes = {
    bookId: PropTypes.number.isRequired,
    lineCount: PropTypes.number,
    apiPrefix: PropTypes.string,
};

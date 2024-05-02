import {useEffect, useRef, useState, Suspense} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";
import {ReactReader} from "react-reader";

export default function ViewEPUB(props) {
    const renditionRef = useRef(null)
    const [url, setUrl] = useState("");
    const [location, setLocation] = useState('')

    useEffect(() => {
        //console.log(`ViewEPUB: useEffect(): props=${JSON.stringify(props)}`);
        if (renditionRef && renditionRef.current) {
            console.log(`renditionRef.current=${renditionRef.current}`);
        }

        const url = getApiUrlPrefix() + "/download/" + props.bookId + "/" + props.filePath;
        setUrl(url);
        //console.log(url);

        return () => {
            setUrl("");
        };
    }, [props]);

    return (
        <div style={{height: "100vh"}}>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <ReactReader
                    location={location}
                    locationChanged={(epubcfi) => setLocation(epubcfi)}
                    url={url}
                    getRendition={(rendition) => {
                        const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
                        rendition.book.spine.get = function (target) {
                            let t = spine_get(target);
                            if (!t) {
                                t = spine_get(undefined);
                            }
                            return t;
                        }
                    }}
                />
            </Suspense>
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string.isRequired,
};
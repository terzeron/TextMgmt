import {useEffect, useState} from "react";
import PropTypes from 'prop-types';
import {getApiUrlPrefix} from "./Common";
import {ReactReader} from "react-reader";

export default function ViewEPUB(props) {
    const [url, setUrl] = useState("");
    const [location, setLocation] = useState(null)

    useEffect(() => {
        const url = getApiUrlPrefix() + "/download/" + props.bookId + "/" + props.filePath;
        setUrl(url);
        return () => {
            setUrl("");
        };
    }, [props]);

    return (
        <div style={{height: "100vh"}}>
            {!url && <div>로딩 중...</div>}
            {url && <ReactReader
                location={location}
                locationChanged={(epubcfi) => setLocation(epubcfi)}
                url={url}
            />}
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string.isRequired,
};
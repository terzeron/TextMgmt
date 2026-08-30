import React from "react";
import PropTypes from "prop-types";
import { Alert, Button, Card, Container } from "react-bootstrap";
import { reportClientError } from "./clientLogger";

class ErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      showDetails: false,
    };
  }

  static getDerivedStateFromError(error) {
    return { hasError: true, error };
  }

  componentDidCatch(error, errorInfo) {
    if (this.props.onError) {
      try {
        this.props.onError(error, errorInfo);
      } catch {
        // 콜백 오류 무음 처리
      }
    }

    reportClientError({
      errorType: "REACT_RENDER_ERROR",
      message: error?.message || String(error),
      stack: error?.stack,
      componentStack: errorInfo?.componentStack,
    });
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null, showDetails: false });
    if (this.props.onReset) {
      try {
        this.props.onReset();
      } catch {
        // 무음 처리
      }
    }
  };

  handleReload = () => {
    if (typeof window !== "undefined" && window.location) {
      window.location.reload();
    }
  };

  handleGoHome = () => {
    if (typeof window !== "undefined" && window.location) {
      window.location.href = "/";
    }
  };

  toggleDetails = () => {
    this.setState((prev) => ({ showDetails: !prev.showDetails }));
  };

  render() {
    if (this.state.hasError) {
      if (typeof this.props.fallback === "function") {
        return this.props.fallback(this.state.error, this.handleReset);
      }
      if (this.props.fallback) {
        return this.props.fallback;
      }

      return (
        <Container className="d-flex justify-content-center align-items-center py-5" data-testid="error-boundary-fallback">
          <Card className="shadow-sm border-danger w-100" style={{ maxWidth: "600px" }}>
            <Card.Header className="bg-danger text-white d-flex align-items-center">
              <span className="me-2" role="img" aria-label="warning">⚠️</span>
              <strong>화면 표시 중 오류가 발생했습니다</strong>
            </Card.Header>
            <Card.Body>
              <Alert variant="warning" className="mb-3">
                예기치 못한 런타임 오류가 발생하여 화면을 표시할 수 없습니다.
                오류 정보는 서버로 자동 보고되었습니다.
              </Alert>

              <div className="d-flex flex-wrap gap-2 mb-3">
                <Button variant="primary" size="sm" onClick={this.handleReset}>
                  다시 시도
                </Button>
                <Button variant="outline-secondary" size="sm" onClick={this.handleReload}>
                  새로고침
                </Button>
                <Button variant="outline-primary" size="sm" onClick={this.handleGoHome}>
                  홈으로 이동
                </Button>
                <Button variant="link" size="sm" className="text-muted p-0 ms-auto" onClick={this.toggleDetails}>
                  {this.state.showDetails ? "상세 정보 숨기기" : "오류 상세 정보"}
                </Button>
              </div>

              {this.state.showDetails && (
                <div className="bg-light p-3 rounded border text-start">
                  <div className="text-danger fw-bold mb-1 small">
                    {this.state.error?.name || "Error"}: {this.state.error?.message || "Unknown error"}
                  </div>
                  {this.state.error?.stack && (
                    <pre
                      className="text-muted small mb-0 overflow-auto"
                      style={{ maxHeight: "180px", fontSize: "0.75rem" }}
                    >
                      {this.state.error.stack}
                    </pre>
                  )}
                </div>
              )}
            </Card.Body>
          </Card>
        </Container>
      );
    }

    return this.props.children;
  }
}

ErrorBoundary.propTypes = {
  children: PropTypes.node,
  fallback: PropTypes.oneOfType([PropTypes.node, PropTypes.func]),
  onError: PropTypes.func,
  onReset: PropTypes.func,
};

export default ErrorBoundary;

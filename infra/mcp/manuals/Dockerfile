FROM python:3.12-slim

ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONUNBUFFERED=1

WORKDIR /app

COPY infra/mcp/manuals/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY infra/mcp/manuals/server.py .
COPY data/knowledge/source /app/knowledge

USER 65532:65532

EXPOSE 8000

CMD ["python", "/app/server.py"]

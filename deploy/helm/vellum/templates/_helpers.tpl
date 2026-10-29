{{/*
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
*/}}

{{- define "vellum.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "vellum.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else if contains (include "vellum.name" .) .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name (include "vellum.name" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "vellum.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: vellum
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}

{{- define "vellum.selector" -}}
app.kubernetes.io/name: {{ include "vellum.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: server
{{- end }}

{{- define "vellum.krokiSelector" -}}
app.kubernetes.io/name: {{ include "vellum.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: kroki
{{- end }}

{{- define "vellum.clamavSelector" -}}
app.kubernetes.io/name: {{ include "vellum.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: clamav
{{- end }}

{{/* The secret holding the shared CSRF key (see csrf-secret.yaml). */}}
{{- define "vellum.csrfSecret" -}}
{{- .Values.csrf.secret.name | default (printf "%s-csrf" (include "vellum.fullname" .)) -}}
{{- end }}

{{- define "vellum.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "vellum.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/* An image reference; a digest wins over the tag. */}}
{{- define "vellum.image" -}}
{{- $tag := .image.tag | default .default -}}
{{- if .image.digest }}
{{- printf "%s:%s@%s" .image.repository $tag .image.digest }}
{{- else }}
{{- printf "%s:%s" .image.repository $tag }}
{{- end }}
{{- end }}

{{/* The Patroni cluster name for a shard; the operator requires the team prefix. */}}
{{- define "vellum.pgCluster" -}}
{{- printf "%s-%s-%s" .root.Values.postgres.operator.teamId .root.Release.Name .shard.name | trunc 50 | trimSuffix "-" }}
{{- end }}

{{/* The shard names, comma-separated, in order. */}}
{{- define "vellum.shardNames" -}}
{{- $names := list }}
{{- range .Values.shards }}
{{- if not (regexMatch "^[a-z][a-z0-9-]{0,14}$" .name) }}
{{- fail (printf "shard name %q must be lower-case letters, digits and dashes" .name) }}
{{- end }}
{{- $names = append $names .name }}
{{- end }}
{{- join "," $names }}
{{- end }}

{{/* Shard names upper-cased for environment variable names. */}}
{{- define "vellum.shardEnv" -}}
{{- .name | upper | replace "-" "_" }}
{{- end }}

{{/* Hardened container security context shared by every container. */}}
{{- define "vellum.containerSecurity" -}}
allowPrivilegeEscalation: false
readOnlyRootFilesystem: true
runAsNonRoot: true
runAsUser: 65532
runAsGroup: 65532
capabilities:
  drop: [ALL]
seccompProfile:
  type: RuntimeDefault
{{- end }}

{{/* The server pod template, shared by the Deployment and the Rollout. */}}
{{- define "vellum.podTemplate" -}}
metadata:
  labels:
    {{- include "vellum.labels" . | nindent 4 }}
    {{- include "vellum.selector" . | nindent 4 }}
  annotations:
    checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
    {{- with .Values.podAnnotations }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
spec:
  serviceAccountName: {{ include "vellum.serviceAccountName" . }}
  automountServiceAccountToken: false
  {{- with .Values.imagePullSecrets }}
  imagePullSecrets:
    {{- toYaml . | nindent 4 }}
  {{- end }}
  terminationGracePeriodSeconds: {{ .Values.terminationGracePeriodSeconds }}
  securityContext:
    runAsNonRoot: true
    runAsUser: 65532
    runAsGroup: 65532
    fsGroup: 65532
    seccompProfile:
      type: RuntimeDefault
  affinity:
    podAntiAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        - topologyKey: kubernetes.io/hostname
          labelSelector:
            matchLabels:
              {{- include "vellum.selector" . | nindent 14 }}
  topologySpreadConstraints:
    - maxSkew: 1
      topologyKey: topology.kubernetes.io/zone
      whenUnsatisfiable: ScheduleAnyway
      labelSelector:
        matchLabels:
          {{- include "vellum.selector" . | nindent 10 }}
  {{- with .Values.nodeSelector }}
  nodeSelector:
    {{- toYaml . | nindent 4 }}
  {{- end }}
  {{- with .Values.tolerations }}
  tolerations:
    {{- toYaml . | nindent 4 }}
  {{- end }}
  containers:
    - name: vellum
      image: {{ include "vellum.image" (dict "image" .Values.image "default" .Chart.AppVersion) }}
      imagePullPolicy: {{ .Values.image.pullPolicy }}
      args: ["serve"]
      ports:
        - name: http
          containerPort: 7700
          protocol: TCP
      envFrom:
        - configMapRef:
            name: {{ include "vellum.fullname" . }}
        {{- with .Values.blob.credentialsSecret }}
        - secretRef:
            name: {{ . }}
        {{- end }}
      env:
        - name: VELLUM_KEK_PASSPHRASE
          valueFrom:
            secretKeyRef:
              name: {{ .Values.encryption.kekSecret.name }}
              key: {{ .Values.encryption.kekSecret.key }}
        - name: VELLUM_CSRF_KEY
          valueFrom:
            secretKeyRef:
              name: {{ include "vellum.csrfSecret" . }}
              key: {{ .Values.csrf.secret.key }}
        {{- range .Values.shards }}
        {{- $env := include "vellum.shardEnv" . }}
        {{- if eq $.Values.postgres.mode "operator" }}
        {{- $cluster := include "vellum.pgCluster" (dict "root" $ "shard" .) }}
        {{- $secret := printf "vellum.%s.credentials.postgresql.acid.zalan.do" $cluster }}
        - name: VELLUM_SHARD_{{ $env }}_DB_USER
          valueFrom:
            secretKeyRef: { name: {{ $secret }}, key: username }
        - name: VELLUM_SHARD_{{ $env }}_DB_PASSWORD
          valueFrom:
            secretKeyRef: { name: {{ $secret }}, key: password }
        - name: VELLUM_SHARD_{{ $env }}_DATABASE_URL
          value: postgres://$(VELLUM_SHARD_{{ $env }}_DB_USER):$(VELLUM_SHARD_{{ $env }}_DB_PASSWORD)@{{ $cluster }}:5432/{{ $.Values.postgres.database }}?sslmode=require
        {{- else }}
        - name: VELLUM_SHARD_{{ $env }}_DATABASE_URL
          valueFrom:
            secretKeyRef: { name: {{ $.Values.postgres.externalSecret }}, key: {{ .name }} }
        {{- end }}
        {{- end }}
        {{- $home := include "vellum.shardEnv" (first .Values.shards) }}
        - name: VELLUM_DATABASE_URL
          value: $(VELLUM_SHARD_{{ $home }}_DATABASE_URL)
      startupProbe:
        httpGet: { path: /healthz, port: http }
        periodSeconds: 1
        failureThreshold: 30
      livenessProbe:
        httpGet: { path: /healthz, port: http }
        periodSeconds: 10
        timeoutSeconds: 2
        failureThreshold: 3
      readinessProbe:
        httpGet: { path: /readyz, port: http }
        periodSeconds: 5
        timeoutSeconds: 2
        failureThreshold: 2
      lifecycle:
        # Lets endpoints drop the pod before the drain starts.
        preStop:
          sleep: { seconds: 5 }
      resources:
        {{- toYaml .Values.resources | nindent 8 }}
      securityContext:
        {{- include "vellum.containerSecurity" . | nindent 8 }}
      volumeMounts:
        - { name: data, mountPath: /data }
        - { name: tmp, mountPath: /tmp }
  volumes:
    # The Organisation profile keeps everything in Postgres and the bucket;
    # the data folder only caches rendered output and is rebuildable.
    - name: data
      emptyDir: { sizeLimit: 512Mi }
    - name: tmp
      emptyDir: { sizeLimit: 256Mi }
{{- end }}

precision mediump float;
varying vec2 vTexCoord;

uniform vec3 color;
uniform sampler2D atlas;

void main() {
    gl_FragColor = vec4(color, 1.0) * texture2D(atlas, vTexCoord).r;
}

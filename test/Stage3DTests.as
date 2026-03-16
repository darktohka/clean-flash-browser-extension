package
{
    import flash.display.Sprite;
    import flash.display.Stage3D;
    import flash.display.StageAlign;
    import flash.display.StageScaleMode;
    import flash.display3D.Context3D;
    import flash.display3D.Context3DProgramType;
    import flash.display3D.Context3DVertexBufferFormat;
    import flash.display3D.Context3DRenderMode;
    import flash.display3D.IndexBuffer3D;
    import flash.display3D.Program3D;
    import flash.display3D.VertexBuffer3D;
    import flash.events.ErrorEvent;
    import flash.events.Event;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.utils.ByteArray;
    import flash.utils.getTimer;

    [SWF(width="800", height="600", backgroundColor="#1e1e2e", frameRate="60")]
    public class Stage3DTests extends Sprite
    {
        // 2D overlay
        private var _log:TextField;
        private var _fpsField:TextField;

        // 3D
        private var _stage3D:Stage3D;
        private var _context3D:Context3D;
        private var _vertexBuffer:VertexBuffer3D;
        private var _indexBuffer:IndexBuffer3D;
        private var _program:Program3D;

        // animation
        private var _angle:Number = 0;
        private var _frameCount:int = 0;
        private var _lastFpsTime:int;

        public function Stage3DTests()
        {
            stage.align = StageAlign.TOP_LEFT;
            stage.scaleMode = StageScaleMode.NO_SCALE;

            createLogField();
            log("Stage3D Test Suite");
            log("==================");
            log("Stage dimensions: " + stage.stageWidth + "x" + stage.stageHeight);
            log("stage3Ds.length: " + stage.stage3Ds.length);

            if (stage.stage3Ds.length == 0) {
                log("ERROR: No Stage3D objects available.");
                return;
            }

            _stage3D = stage.stage3Ds[0];
            _stage3D.addEventListener(Event.CONTEXT3D_CREATE, onContext3DCreated);
            _stage3D.addEventListener(ErrorEvent.ERROR, onContext3DError);

            log("Requesting Context3D (auto mode)...");
            _stage3D.requestContext3D(Context3DRenderMode.AUTO);
        }

        // ── Logging ──

        private function createLogField():void
        {
            var fmt:TextFormat = new TextFormat("_typewriter", 12, 0xCDD6F4);

            _log = new TextField();
            _log.defaultTextFormat = fmt;
            _log.autoSize = TextFieldAutoSize.LEFT;
            _log.multiline = true;
            _log.wordWrap = true;
            _log.width = 780;
            _log.x = 10;
            _log.y = 10;
            _log.selectable = true;
            _log.mouseEnabled = true;
            addChild(_log);

            _fpsField = new TextField();
            _fpsField.defaultTextFormat = new TextFormat("_typewriter", 14, 0xA6E3A1);
            _fpsField.autoSize = TextFieldAutoSize.RIGHT;
            _fpsField.x = 680;
            _fpsField.y = 10;
            _fpsField.width = 110;
            addChild(_fpsField);
        }

        private function log(msg:String):void
        {
            trace("[Stage3D] " + msg);
            _log.appendText(msg + "\n");
        }

        // ── Context3D creation callbacks ──

        private function onContext3DError(e:ErrorEvent):void
        {
            log("ERROR creating Context3D: " + e.text);
            log("errorID: " + e.errorID);
        }

        private function onContext3DCreated(e:Event):void
        {
            _context3D = _stage3D.context3D;
            if (!_context3D) {
                log("ERROR: context3D is null after creation event.");
                return;
            }

            log("Context3D created successfully!");
            log("  driverInfo: " + _context3D.driverInfo);
            log("  profile: " + _context3D.profile);

            // Configure back buffer: 800x600, no antialiasing, no depth/stencil (simple test)
            _context3D.configureBackBuffer(800, 600, 0, true);
            log("  Back buffer configured: 800x600");

            setupGeometry();
            setupShaders();

            _lastFpsTime = getTimer();
            addEventListener(Event.ENTER_FRAME, onEnterFrame);
            log("Rendering started. You should see a rotating triangle.");
        }

        // ── Geometry: a colored triangle ──

        private function setupGeometry():void
        {
            // 3 vertices: x, y, z, r, g, b
            var verts:Vector.<Number> = new <Number>[
                 0.0,  0.8,  0.0,   0.95, 0.55, 0.66,  // top - red/pink
                -0.8, -0.6,  0.0,   0.58, 0.84, 0.65,  // bottom-left - green
                 0.8, -0.6,  0.0,   0.53, 0.66, 0.98   // bottom-right - blue
            ];

            _vertexBuffer = _context3D.createVertexBuffer(3, 6); // 3 verts, 6 floats each
            _vertexBuffer.uploadFromVector(verts, 0, 3);

            var idx:Vector.<uint> = new <uint>[0, 1, 2];
            _indexBuffer = _context3D.createIndexBuffer(3);
            _indexBuffer.uploadFromVector(idx, 0, 3);
        }

        // ── Shaders (AGAL assembly) ──

        private function setupShaders():void
        {
            // Vertex shader:
            //   op = m44(va0, vc0)   - transform position by matrix in vc0..vc3
            //   v0 = va1             - pass color to fragment
            var vertSrc:ByteArray = assembleAGAL(true,
                "m44 op, va0, vc0\n" +
                "mov v0, va1\n"
            );

            // Fragment shader:
            //   oc = v0  - output interpolated color
            var fragSrc:ByteArray = assembleAGAL(false,
                "mov oc, v0\n"
            );

            _program = _context3D.createProgram();
            _program.upload(vertSrc, fragSrc);
        }

        // ── Minimal AGAL assembler ──

        private function assembleAGAL(isVertex:Boolean, src:String):ByteArray
        {
            var ba:ByteArray = new ByteArray();
            ba.endian = "littleEndian";

            // AGAL header
            ba.writeByte(0xa0);          // magic
            ba.writeUnsignedInt(1);      // version
            ba.writeByte(0xa1);          // shader type tag
            ba.writeByte(isVertex ? 0 : 1); // 0=vertex, 1=fragment

            var lines:Array = src.split("\n");
            for each (var line:String in lines) {
                line = trim(line);
                if (line.length == 0) continue;
                encodeInstruction(ba, line);
            }
            ba.position = 0;
            return ba;
        }

        private function encodeInstruction(ba:ByteArray, line:String):void
        {
            // Supported instructions: mov, m44
            var parts:Array = line.split(/[\s,]+/);
            var op:String = parts[0].toLowerCase();

            var opcodes:Object = {
                "mov": 0x00,
                "m44": 0x18
            };

            if (!(op in opcodes)) {
                log("AGAL: unknown opcode: " + op);
                return;
            }

            ba.writeUnsignedInt(opcodes[op]); // opcode (32-bit)

            // Destination register
            encodeDestination(ba, parts[1]);
            // Source 1
            encodeSource(ba, parts[2]);
            // Source 2 (or zero for single-source ops)
            if (parts.length > 3) {
                encodeSource(ba, parts[3]);
            } else {
                // Zero second source
                ba.writeShort(0);
                ba.writeByte(0);
                ba.writeUnsignedInt(0);
                ba.writeByte(0);
            }
        }

        private function encodeDestination(ba:ByteArray, reg:String):void
        {
            var info:Object = parseReg(reg);
            ba.writeShort(info.index);    // register index (16-bit)
            ba.writeByte(0x0F);           // write mask (xyzw)
            ba.writeByte(info.type);      // register type
        }

        private function encodeSource(ba:ByteArray, reg:String):void
        {
            var info:Object = parseReg(reg);
            ba.writeShort(info.index);    // register index
            ba.writeByte(0);              // indirect offset
            ba.writeByte(info.swizzle);   // swizzle
            ba.writeByte(info.type);      // register type
            ba.writeByte(0);              // index type
            ba.writeShort(0);             // padding
        }

        private function parseReg(name:String):Object
        {
            name = name.replace(/[,\s]/g, "");
            // AGAL register file IDs. These IDs are shared by vertex/fragment
            // profiles, so fragment regs map to the same core types.
            //   attribute=0, constant=1, temporary=2, output=3, varying=4, sampler=5
            var types:Object = {
                "va": 0x00,
                "vc": 0x01,
                "vt": 0x02,
                "op": 0x03,
                "v": 0x04,
                "fc": 0x01,
                "ft": 0x02,
                "oc": 0x03,
                "fs": 0x05
            };

            // Match longest prefixes first so "va0" never gets parsed as "v0".
            var prefixes:Array = ["va", "vc", "vt", "op", "oc", "fc", "ft", "fs", "v"];

            var swizzle:int = 0xE4; // default xyzw = 0b11_10_01_00

            // Extract prefix and index
            for each (var prefix:String in prefixes) {
                if (name.indexOf(prefix) == 0) {
                    var idx:int = parseInt(name.substr(prefix.length));
                    if (isNaN(idx)) idx = 0;
                    return {type: types[prefix], index: idx, swizzle: swizzle};
                }
            }

            log("AGAL: unknown register: " + name);
            return {type: 0, index: 0, swizzle: swizzle};
        }

        private function trim(s:String):String
        {
            return s.replace(/^\s+|\s+$/g, "");
        }

        // ── Render loop ──

        private function onEnterFrame(e:Event):void
        {
            if (!_context3D) return;

            _angle += 1.5;
            _frameCount++;

            // Update FPS every second
            var now:int = getTimer();
            if (now - _lastFpsTime >= 1000) {
                var fps:Number = _frameCount / ((now - _lastFpsTime) / 1000);
                _fpsField.text = fps.toFixed(1) + " FPS";
                _frameCount = 0;
                _lastFpsTime = now;
            }

            // Build a simple rotation matrix around Z axis
            var rad:Number = _angle * Math.PI / 180;
            var c:Number = Math.cos(rad);
            var s:Number = Math.sin(rad);

            // Column-major 4x4 rotation matrix
            var mat:Vector.<Number> = new <Number>[
                c,  s, 0, 0,
               -s,  c, 0, 0,
                0,  0, 1, 0,
                0,  0, 0, 1
            ];

            _context3D.clear(0.118, 0.118, 0.180, 1.0); // #1e1e2e background

            _context3D.setProgram(_program);
            _context3D.setProgramConstantsFromVector(
                Context3DProgramType.VERTEX, 0, mat, 4
            );

            _context3D.setVertexBufferAt(0, _vertexBuffer, 0, Context3DVertexBufferFormat.FLOAT_3); // position
            _context3D.setVertexBufferAt(1, _vertexBuffer, 3, Context3DVertexBufferFormat.FLOAT_3); // color

            _context3D.drawTriangles(_indexBuffer);
            _context3D.present();
        }
    }
}

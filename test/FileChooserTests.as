package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.text.TextFieldType;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.events.SecurityErrorEvent;
    import flash.events.ProgressEvent;
    import flash.events.MouseEvent;
    import flash.net.FileFilter;
    import flash.net.FileReference;
    import flash.net.FileReferenceList;
    import flash.utils.ByteArray;

    [SWF(width="800", height="600", backgroundColor="#1e1e2e", frameRate="30")]
    public class FileChooserTests extends Sprite
    {
        // Colors
        private static const COLOR_BG:uint         = 0x1e1e2e;
        private static const COLOR_HEADER_BG:uint   = 0x181825;
        private static const COLOR_BUTTON:uint      = 0x313244;
        private static const COLOR_BUTTON_HOVER:uint = 0x45475a;
        private static const COLOR_TEXT:uint         = 0xcdd6f4;
        private static const COLOR_DIM:uint          = 0x6c7086;
        private static const COLOR_LOG_BG:uint       = 0x11111b;
        private static const COLOR_SUCCESS:uint      = 0xa6e3a1;
        private static const COLOR_ERROR:uint        = 0xf38ba8;
        private static const COLOR_INFO:uint         = 0x89b4fa;

        private var logField:TextField;
        private var fileRef:FileReference;
        private var fileRefList:FileReferenceList;
        private var logContent:String = "";
        private var buttonY:int = 50;

        public function FileChooserTests()
        {
            addEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
        }

        private function onAddedToStage(e:Event):void
        {
            removeEventListener(Event.ADDED_TO_STAGE, onAddedToStage);

            fileRef = new FileReference();
            fileRefList = new FileReferenceList();

            setupFileRefListeners(fileRef);
            setupFileRefListListeners(fileRefList);

            drawUI();
        }

        private function setupFileRefListeners(fr:FileReference):void
        {
            fr.addEventListener(Event.SELECT, onFileSelect);
            fr.addEventListener(Event.CANCEL, onFileCancel);
            fr.addEventListener(Event.COMPLETE, onFileComplete);
            fr.addEventListener(Event.OPEN, onFileOpen);
            fr.addEventListener(ProgressEvent.PROGRESS, onFileProgress);
            fr.addEventListener(IOErrorEvent.IO_ERROR, onFileIOError);
            fr.addEventListener(SecurityErrorEvent.SECURITY_ERROR, onFileSecurityError);
        }

        private function setupFileRefListListeners(frl:FileReferenceList):void
        {
            frl.addEventListener(Event.SELECT, onFileListSelect);
            frl.addEventListener(Event.CANCEL, onFileCancel);
        }

        private function drawUI():void
        {
            // Background
            var bg:Shape = new Shape();
            bg.graphics.beginFill(COLOR_BG);
            bg.graphics.drawRect(0, 0, 800, 600);
            bg.graphics.endFill();
            addChild(bg);

            // Header
            var header:Shape = new Shape();
            header.graphics.beginFill(COLOR_HEADER_BG);
            header.graphics.drawRect(0, 0, 800, 40);
            header.graphics.endFill();
            addChild(header);

            var titleFmt:TextFormat = new TextFormat("_sans", 14, COLOR_TEXT, true);
            var title:TextField = new TextField();
            title.defaultTextFormat = titleFmt;
            title.text = "FileReference / FileReferenceList Test Suite";
            title.autoSize = TextFieldAutoSize.LEFT;
            title.selectable = false;
            title.x = 10;
            title.y = 10;
            addChild(title);

            // Buttons column
            buttonY = 50;

            createButton("Browse Single File (All)", function(e:MouseEvent):void {
                appendLog("Calling FileReference.browse() with no filter...", COLOR_INFO);
                fileRef.browse();
            });

            createButton("Browse Single (Text Files)", function(e:MouseEvent):void {
                var filter:FileFilter = new FileFilter("Text Files", "*.txt;*.rtf;*.csv");
                appendLog("Calling FileReference.browse([TextFilter])...", COLOR_INFO);
                fileRef.browse([filter]);
            });

            createButton("Browse Single (Images)", function(e:MouseEvent):void {
                var filter:FileFilter = new FileFilter("Images", "*.jpg;*.jpeg;*.png;*.gif;*.bmp");
                appendLog("Calling FileReference.browse([ImageFilter])...", COLOR_INFO);
                fileRef.browse([filter]);
            });

            createButton("Browse Single (Multiple Filters)", function(e:MouseEvent):void {
                var textFilter:FileFilter = new FileFilter("Text Files", "*.txt;*.rtf");
                var imageFilter:FileFilter = new FileFilter("Images", "*.jpg;*.png;*.gif");
                var allFilter:FileFilter = new FileFilter("All Files", "*.*");
                appendLog("Calling FileReference.browse([Text, Images, All])...", COLOR_INFO);
                fileRef.browse([textFilter, imageFilter, allFilter]);
            });

            createButton("Browse Multiple Files", function(e:MouseEvent):void {
                appendLog("Calling FileReferenceList.browse() with no filter...", COLOR_INFO);
                fileRefList.browse();
            });

            createButton("Browse Multiple (Images)", function(e:MouseEvent):void {
                var filter:FileFilter = new FileFilter("Images", "*.jpg;*.jpeg;*.png;*.gif;*.bmp");
                appendLog("Calling FileReferenceList.browse([ImageFilter])...", COLOR_INFO);
                fileRefList.browse([filter]);
            });

            createButton("Browse + Load File Data", function(e:MouseEvent):void {
                appendLog("Calling FileReference.browse() then will load()...", COLOR_INFO);
                var loadRef:FileReference = new FileReference();
                loadRef.addEventListener(Event.SELECT, function(se:Event):void {
                    appendLog("File selected, calling load()...", COLOR_INFO);
                    loadRef.load();
                });
                loadRef.addEventListener(Event.COMPLETE, function(ce:Event):void {
                    var data:ByteArray = loadRef.data;
                    appendLog("File loaded! Size: " + data.length + " bytes", COLOR_SUCCESS);
                    // Show first 100 bytes as string preview
                    if (data.length > 0) {
                        data.position = 0;
                        var preview:String = data.readUTFBytes(Math.min(data.length, 100));
                        appendLog("Preview: " + preview.substr(0, 80) + (data.length > 80 ? "..." : ""), COLOR_DIM);
                    }
                });
                loadRef.addEventListener(Event.CANCEL, onFileCancel);
                loadRef.addEventListener(IOErrorEvent.IO_ERROR, onFileIOError);
                loadRef.browse();
            });

            createButton("Save File (save())", function(e:MouseEvent):void {
                appendLog("Calling FileReference.save() with text data...", COLOR_INFO);
                var saveRef:FileReference = new FileReference();
                saveRef.addEventListener(Event.SELECT, function(se:Event):void {
                    appendLog("Save location selected: " + saveRef.name, COLOR_SUCCESS);
                });
                saveRef.addEventListener(Event.COMPLETE, function(ce:Event):void {
                    appendLog("File saved successfully!", COLOR_SUCCESS);
                });
                saveRef.addEventListener(Event.CANCEL, function(ce:Event):void {
                    appendLog("Save cancelled by user", COLOR_DIM);
                });
                saveRef.addEventListener(IOErrorEvent.IO_ERROR, onFileIOError);
                var testData:String = "Hello from Flash FileReference.save() test!\n" +
                    "This is test content generated at runtime.\n" +
                    "Timestamp: " + new Date().toString();
                saveRef.save(testData, "test_output.txt");
            });

            createButton("Download File", function(e:MouseEvent):void {
                appendLog("Calling FileReference.download()...", COLOR_INFO);
                var dlRef:FileReference = new FileReference();
                dlRef.addEventListener(Event.SELECT, function(se:Event):void {
                    appendLog("Download save location selected", COLOR_INFO);
                });
                dlRef.addEventListener(Event.COMPLETE, function(ce:Event):void {
                    appendLog("Download completed: " + dlRef.name + " (" + dlRef.size + " bytes)", COLOR_SUCCESS);
                });
                dlRef.addEventListener(Event.CANCEL, function(ce:Event):void {
                    appendLog("Download cancelled by user", COLOR_DIM);
                });
                dlRef.addEventListener(IOErrorEvent.IO_ERROR, onFileIOError);
                dlRef.addEventListener(SecurityErrorEvent.SECURITY_ERROR, onFileSecurityError);
                var req:flash.net.URLRequest = new flash.net.URLRequest("http://localhost:3000/text");
                dlRef.download(req, "downloaded.txt");
            });

            createButton("Clear Log", function(e:MouseEvent):void {
                logContent = "";
                logField.text = "";
            });

            // Log area
            var logBg:Shape = new Shape();
            logBg.graphics.beginFill(COLOR_LOG_BG);
            logBg.graphics.drawRect(300, 50, 490, 540);
            logBg.graphics.endFill();
            addChild(logBg);

            var logLabelFmt:TextFormat = new TextFormat("_sans", 11, COLOR_DIM, true);
            var logLabel:TextField = new TextField();
            logLabel.defaultTextFormat = logLabelFmt;
            logLabel.text = "Event Log";
            logLabel.autoSize = TextFieldAutoSize.LEFT;
            logLabel.selectable = false;
            logLabel.x = 305;
            logLabel.y = 52;
            addChild(logLabel);

            var logFmt:TextFormat = new TextFormat("_typewriter", 11, COLOR_TEXT);
            logField = new TextField();
            logField.defaultTextFormat = logFmt;
            logField.x = 305;
            logField.y = 70;
            logField.width = 480;
            logField.height = 515;
            logField.multiline = true;
            logField.wordWrap = true;
            logField.selectable = true;
            addChild(logField);

            appendLog("FileChooser Test Suite ready.", COLOR_SUCCESS);
            appendLog("Click a button to test file chooser APIs.", COLOR_DIM);
        }

        private function createButton(label:String, handler:Function):void
        {
            var btn:Sprite = new Sprite();
            btn.graphics.beginFill(COLOR_BUTTON);
            btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
            btn.graphics.endFill();
            btn.x = 10;
            btn.y = buttonY;
            btn.buttonMode = true;
            btn.useHandCursor = true;

            var fmt:TextFormat = new TextFormat("_sans", 11, COLOR_TEXT);
            var tf:TextField = new TextField();
            tf.defaultTextFormat = fmt;
            tf.text = label;
            tf.autoSize = TextFieldAutoSize.LEFT;
            tf.selectable = false;
            tf.mouseEnabled = false;
            tf.x = 10;
            tf.y = 7;
            btn.addChild(tf);

            btn.addEventListener(MouseEvent.CLICK, handler);
            btn.addEventListener(MouseEvent.ROLL_OVER, function(e:MouseEvent):void {
                btn.graphics.clear();
                btn.graphics.beginFill(COLOR_BUTTON_HOVER);
                btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
                btn.graphics.endFill();
            });
            btn.addEventListener(MouseEvent.ROLL_OUT, function(e:MouseEvent):void {
                btn.graphics.clear();
                btn.graphics.beginFill(COLOR_BUTTON);
                btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
                btn.graphics.endFill();
            });

            addChild(btn);
            buttonY += 38;
        }

        // FileReference event handlers
        private function onFileSelect(e:Event):void
        {
            var fr:FileReference = e.target as FileReference;
            appendLog("SELECT: " + fr.name, COLOR_SUCCESS);
            appendLog("  Type: " + fr.type + ", Size: " + fr.size + " bytes", COLOR_DIM);
            try {
                appendLog("  Created: " + fr.creationDate, COLOR_DIM);
                appendLog("  Modified: " + fr.modificationDate, COLOR_DIM);
            } catch (err:Error) {
                // Some properties may not be available
            }
        }

        private function onFileCancel(e:Event):void
        {
            appendLog("CANCEL: User cancelled the file dialog", COLOR_DIM);
        }

        private function onFileComplete(e:Event):void
        {
            var fr:FileReference = e.target as FileReference;
            appendLog("COMPLETE: " + fr.name, COLOR_SUCCESS);
        }

        private function onFileOpen(e:Event):void
        {
            appendLog("OPEN: File operation started", COLOR_INFO);
        }

        private function onFileProgress(e:ProgressEvent):void
        {
            appendLog("PROGRESS: " + e.bytesLoaded + "/" + e.bytesTotal, COLOR_DIM);
        }

        private function onFileIOError(e:IOErrorEvent):void
        {
            appendLog("IO_ERROR: " + e.text, COLOR_ERROR);
        }

        private function onFileSecurityError(e:SecurityErrorEvent):void
        {
            appendLog("SECURITY_ERROR: " + e.text, COLOR_ERROR);
        }

        // FileReferenceList event handler
        private function onFileListSelect(e:Event):void
        {
            var frl:FileReferenceList = e.target as FileReferenceList;
            appendLog("MULTI-SELECT: " + frl.fileList.length + " files chosen", COLOR_SUCCESS);
            for (var i:int = 0; i < frl.fileList.length; i++) {
                var f:FileReference = frl.fileList[i];
                appendLog("  [" + i + "] " + f.name + " (" + f.size + " bytes, type: " + f.type + ")", COLOR_DIM);
            }
        }

        private function appendLog(msg:String, color:uint = 0xcdd6f4):void
        {
            trace("[FileChooserTest] " + msg);
            var fmt:TextFormat = new TextFormat("_typewriter", 11, color);
            var startIdx:int = logField.text.length;
            logField.appendText(msg + "\n");
            logField.setTextFormat(fmt, startIdx, logField.text.length);
            logField.scrollV = logField.maxScrollV;
        }
    }
}
